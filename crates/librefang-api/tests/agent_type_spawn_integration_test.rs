//! Integration tests for two fixes shipped together for the "Agent Types
//! page is uneditable" bug (an install with 42 agents and 0 templates had
//! every agent listed as an unusable agent type):
//!
//!  1. `POST /api/agents {"template": name}` now resolves a real
//!     `~/.librefang/agent-types/<name>.toml` template FIRST. Before this
//!     fix `resolve_manifest` (`routes/agents/lifecycle.rs`) only ever
//!     checked `workspaces/agents/<name>/agent.toml` — spawning a
//!     persistent agent "from template" 404'd for every real template and
//!     only worked when `name` happened to match an existing agent's own
//!     workspace, the opposite of what the endpoint promises. The
//!     workspace fallback is kept second, for backward compatibility.
//!  2. `POST /api/agents/{id}/save-as-agent-type` extracts a live agent's
//!     manifest into a reusable `agent-types/<name>.toml` file — the bridge
//!     from "I have agents, not templates" to a populated, editable Agent
//!     Types page.
//!
//! `LIBREFANG_HOME` and `KernelConfig::home_dir` are pinned to the SAME
//! tempdir per test: `agent_templates.rs`'s reads/writes
//! (`GET/POST/PUT/DELETE /api/templates`) resolve `agent-types/` through
//! `LIBREFANG_HOME`, while the kernel's own template-dir fallback resolves
//! it through `KernelConfig::home_dir` — the two have to agree for a file
//! this test writes (or the save-as-agent-type handler writes) to be
//! visible to both surfaces. Env var mutation is process-global, so every
//! test in this file is serialised behind `home_lock()`.
//!
//! Run: cargo test -p librefang-api --test agent_type_spawn_integration_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest, ModelConfig};
use librefang_types::config::{DefaultModelConfig, KernelConfig, UserConfig};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

/// Serialises every test in this file — `LIBREFANG_HOME` is a process-wide
/// env var, and each test needs it pinned to its own fresh tempdir.
fn home_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

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

/// Boots a fresh kernel + production router with `LIBREFANG_HOME` and
/// `KernelConfig::home_dir` pinned to the same fresh tempdir. Caller must
/// hold `home_lock()` for the duration of the harness's use.
async fn boot() -> Harness {
    boot_with_users(&[]).await
}

/// Like [`boot`], plus the given `(name, role, api_key)` RBAC users.
///
/// The per-user bearer table is derived from `KernelConfig::users` when the
/// production router is built, so a test that needs a real role gate must
/// declare the user before boot rather than injecting the extension later.
async fn boot_with_users(users: &[(&str, &str, &str)]) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Safety: env mutation, serialised by `home_lock()`.
    std::env::set_var("LIBREFANG_HOME", tmp.path());

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let user_configs: Vec<UserConfig> = users
        .iter()
        .map(|(name, role, key)| UserConfig {
            name: (*name).to_string(),
            role: (*role).to_string(),
            channel_bindings: std::collections::HashMap::new(),
            api_key_hash: Some(
                librefang_api::password_hash::hash_password(key).expect("hash user api key"),
            ),
            ..Default::default()
        })
        .collect();

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        users: user_configs,
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

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
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

fn minimal_manifest_toml(name: &str) -> String {
    format!(
        r#"name = "{name}"
description = "test template"

[model]
provider = "default"
model = "default"
"#
    )
}

// ---------------------------------------------------------------------------
// Point 2: POST /api/agents {"template": name} resolves real templates
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn spawn_from_real_template_succeeds() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let templates_dir = h.state.kernel.config_ref().home_dir.join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("real-template.toml"),
        minimal_manifest_toml("real-template"),
    )
    .unwrap();

    // Before the fix: this 404'd — `resolve_manifest` never looked in
    // `templates/` at all, only in `workspaces/agents/`.
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({"template": "real-template"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(body["agent_id"].is_string(), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_from_workspace_agent_name_still_works() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    // Backward-compat fallback (kept deliberately, point 4 of the fix): a
    // "template" name that matches an existing agent's own workspace
    // directory still resolves, now as the second-choice path behind
    // `templates/`.
    let agent_dir = h
        .state
        .kernel
        .config_ref()
        .home_dir
        .join("workspaces")
        .join("agents")
        .join("workspace-source");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("agent.toml"),
        minimal_manifest_toml("workspace-source"),
    )
    .unwrap();

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({"template": "workspace-source"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_from_template_prefers_templates_dir_over_workspace_agent() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    // Same name in both places — `agent-types/` must win, matching the
    // precedence `librefang_kernel::agent_template::load_agent_template`
    // (this route's template loader) already uses: `agent-types/` →
    // `workspaces/agents/` → `registry/agents/`.
    let home = h.state.kernel.config_ref().home_dir.clone();
    let templates_dir = home.join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("shadowed-name.toml"),
        r#"name = "shadowed-name"
description = "from templates dir"

[model]
provider = "default"
model = "default"
"#,
    )
    .unwrap();
    let agent_dir = home.join("workspaces").join("agents").join("shadowed-name");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("agent.toml"),
        r#"name = "shadowed-name"
description = "from workspace agent"

[model]
provider = "default"
model = "default"
"#,
    )
    .unwrap();

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({"template": "shadowed-name", "name": "spawned-from-shadowed"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let agent_id: AgentId = body["agent_id"]
        .as_str()
        .expect("agent_id")
        .parse()
        .unwrap();
    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("spawned entry");
    assert_eq!(entry.manifest.description, "from templates dir");
}

// ---------------------------------------------------------------------------
// Point 3: POST /api/agents/{id}/save-as-agent-type + round trip
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_round_trip() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    // Spawn a live agent with a distinctive config to snapshot.
    let manifest = AgentManifest {
        name: "researcher-live".to_string(),
        description: "Deep research specialist".to_string(),
        skills: vec!["web-research".to_string()],
        model: ModelConfig {
            provider: "test-provider".to_string(),
            model: "test-model".to_string(),
            system_prompt: "You are a researcher.".to_string(),
            ..Default::default()
        },
        ..AgentManifest::default()
    };
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent");

    // Save it as an agent-type template under a DIFFERENT name — Clone
    // would duplicate the running instance; this snapshots its config
    // into a reusable template instead, without touching the source agent.
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{agent_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "researcher-template"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["name"], "researcher-template");
    assert_eq!(body["description"], "Deep research specialist");

    // The template file must exist and its `workspace` must be cleared —
    // otherwise spawning from it later would point the new agent at the
    // SOURCE agent's own workspace directory instead of a fresh one.
    let templates_dir = h.state.kernel.config_ref().home_dir.join("agent-types");
    let content = std::fs::read_to_string(templates_dir.join("researcher-template.toml"))
        .expect("template file must exist on disk");
    let saved: AgentManifest = toml::from_str(&content).unwrap();
    assert_eq!(saved.name, "researcher-template");
    assert!(
        saved.workspace.is_none(),
        "workspace must be cleared on save: {content}"
    );
    // `agent_purge` reads this to leave a same-named type alone; without it the
    // purge would delete the operator's copy when the source agent is removed.
    assert_eq!(
        saved
            .metadata
            .get(librefang_types::agent_type_store::SAVED_FROM_AGENT_METADATA_KEY)
            .and_then(|value| value.as_str()),
        Some("researcher-live"),
        "the snapshot must record its source agent for agent_purge: {content}"
    );

    // It shows up on the Agent Types list, editable/deletable like any
    // other template (point 1: only real templates are listed).
    let (list_status, list_body) = send(h.app.clone(), get("/api/templates")).await;
    assert_eq!(list_status, StatusCode::OK, "{list_body}");
    let items = list_body["templates"].as_array().expect("templates array");
    assert!(
        items
            .iter()
            .any(|i| i["name"] == "researcher-template" && i["source"] == "agent-type"),
        "saved template must be listed as an editable agent type: {list_body}"
    );

    // Round trip: spawn a brand new agent from the saved template.
    let (spawn_status, spawn_body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({"template": "researcher-template", "name": "researcher-clone"}),
        ),
    )
    .await;
    assert_eq!(spawn_status, StatusCode::CREATED, "{spawn_body}");
    let new_id: AgentId = spawn_body["agent_id"]
        .as_str()
        .expect("agent_id")
        .parse()
        .unwrap();
    let new_entry = h
        .state
        .kernel
        .agent_registry()
        .get(new_id)
        .expect("spawned entry");
    assert_eq!(new_entry.manifest.description, "Deep research specialist");
    assert_eq!(new_entry.manifest.skills, vec!["web-research".to_string()]);

    // The decisive check is the saved TOML's own text: a `workspace` key
    // would point every agent spawned from it at the SOURCE's directory.
    // Comparing the two agents' assigned workspaces proves nothing — those
    // differ by name even when the snapshot carried the key.
    let saved_raw: toml::Value = toml::from_str(&content).unwrap();
    assert!(
        saved_raw.get("workspace").is_none(),
        "the saved template must not carry a workspace key: {content}"
    );
    assert!(
        new_entry.manifest.workspace.is_some(),
        "the spawned agent must still get its own workspace from spawn"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_strips_kernel_hand_tags() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let mut source = AgentManifest {
        name: "hand-worker".to_string(),
        tags: vec![
            "hand:researcher".to_string(),
            "hand_role:lead".to_string(),
            "hand_instance:abc".to_string(),
            "operator-tag".to_string(),
        ],
        ..AgentManifest::default()
    };
    // The hand lifecycle stamps this on the live agent. The runtime reads it
    // for EVERY agent, so a snapshot that carried it would hand a plain
    // agent's shell/subprocess tools the hand's frozen passthrough list.
    source.metadata.insert(
        "hand_allowed_env".to_string(),
        serde_json::json!(["HAND_INSTANCE_TOKEN"]),
    );
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(source)
        .expect("spawn_agent");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{agent_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "snapshotted-hand"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Kernel-owned tags must not travel into the saved type: a copied `hand:*`
    // tag makes `spawn` recompute `is_hand`, and the approval gate then
    // auto-approves every tool call for agents spawned from it.
    let templates_dir = h.state.kernel.config_ref().home_dir.join("agent-types");
    let content = std::fs::read_to_string(templates_dir.join("snapshotted-hand.toml"))
        .expect("template file must exist on disk");
    let saved: AgentManifest = toml::from_str(&content).unwrap();
    assert_eq!(
        saved.tags,
        vec!["operator-tag".to_string()],
        "only operator tags may be snapshotted: {content}"
    );
    // Hand-owned metadata must be stripped at least as thoroughly as the
    // kernel-owned tags: the key must not be in the written TOML.
    assert!(
        !saved.metadata.contains_key("hand_allowed_env"),
        "a hand's env passthrough allowlist must not be snapshotted: {content}"
    );

    // The decisive check: an agent spawned from the snapshot is not a hand.
    let (spawn_status, spawn_body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({"template": "snapshotted-hand", "name": "plain-copy"}),
        ),
    )
    .await;
    assert_eq!(spawn_status, StatusCode::CREATED, "{spawn_body}");
    let new_id: AgentId = spawn_body["agent_id"]
        .as_str()
        .expect("agent_id")
        .parse()
        .unwrap();
    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(new_id)
        .expect("spawned entry");
    assert!(
        !entry.is_hand,
        "a snapshot must not spawn hands: {:?}",
        entry.manifest.tags
    );
    assert!(
        !entry.tags.iter().any(|t| t.starts_with("hand:")),
        "no hand tag may reach the spawned agent: {:?}",
        entry.tags
    );
    // The decisive metadata check: an agent spawned from the snapshot must
    // not inherit the hand's env passthrough allowlist.
    assert!(
        !entry.manifest.metadata.contains_key("hand_allowed_env"),
        "no hand-owned metadata may reach the spawned agent: {:?}",
        entry.manifest.metadata
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_rejects_duplicate_template_name() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let templates_dir = h.state.kernel.config_ref().home_dir.join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("existing-template.toml"),
        minimal_manifest_toml("existing-template"),
    )
    .unwrap();

    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "any-agent".to_string(),
            ..AgentManifest::default()
        })
        .unwrap();

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{agent_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "existing-template"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_allows_the_source_agents_own_name() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let source_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "self-named".to_string(),
            ..AgentManifest::default()
        })
        .unwrap();
    let other_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "other-agent".to_string(),
            ..AgentManifest::default()
        })
        .unwrap();

    // Saving under the SOURCE agent's own name is fine — the common "make
    // this agent's config reusable" case.
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{source_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "self-named"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Re-saving over the agent type the call above just wrote is refused by
    // the `NameTaken` claim. This asserts only that the file is not silently
    // replaced; the shadow guard is a separate concern, covered by
    // `save_agent_as_agent_type_refuses_to_shadow_a_different_live_agent` —
    // which orders things so `NameTaken` cannot fire and mask its absence.
    let (status2, body2) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{other_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "self-named"}),
        ),
    )
    .await;
    assert_eq!(status2, StatusCode::CONFLICT, "{body2}");
}

/// Saving one agent under a DIFFERENT live agent's name must be refused.
///
/// `create_agent_type` already refuses this with `ShadowsLiveAgent`, and its
/// stated reason is that an agent type sharing a live agent's name wins every
/// subsequent catalog read and leaves the agent unreachable. Point 1 of this
/// change makes that worse rather than better: `resolve_manifest` now prefers
/// the agent-type store, so the shadowing type wins outright.
///
/// The ordering matters. The sibling test above creates the agent type first
/// and then re-saves over it, which is refused by the `NameTaken` claim and
/// would pass even with no shadow check at all. Here the victim is a live
/// agent with NO agent type of its name, so `NameTaken` cannot fire and only a
/// real shadow check can produce the refusal.
#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_refuses_to_shadow_a_different_live_agent() {
    let _guard = home_lock().lock().await;
    let h = boot().await;
    let home = h.state.kernel.config_ref().home_dir.clone();

    let victim_dir = home.join("workspaces").join("agents").join("victim");
    std::fs::create_dir_all(&victim_dir).unwrap();
    std::fs::write(
        victim_dir.join("agent.toml"),
        minimal_manifest_toml("victim"),
    )
    .unwrap();

    let other_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "unrelated".to_string(),
            ..AgentManifest::default()
        })
        .unwrap();

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{other_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "victim"}),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "saving 'unrelated' under live agent 'victim''s name must be refused: {body}"
    );
    assert!(
        !librefang_types::agent_type_store::agent_type_path_in(&home, "victim").exists(),
        "a refused save must not leave an agent-type file behind"
    );
}

/// The handler's doc argues it needs no per-agent ownership check because
/// `save-as-agent-type` is absent from the `User`-tier POST allowlist, so
/// only Admin+ callers reach it at all. Pin that with a real `User` bearer
/// through the production middleware stack.
#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_forbids_user_role() {
    let _guard = home_lock().lock().await;
    let h = boot_with_users(&[("Alice", "user", "alice-user-key")]).await;

    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "user-save-source".to_string(),
            ..AgentManifest::default()
        })
        .unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/agents/{agent_id}/save-as-agent-type"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer alice-user-key")
        .body(Body::from(
            serde_json::json!({"template_name": "user-save"}).to_string(),
        ))
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a User bearer must not reach the handler: {body}"
    );

    let home = h.state.kernel.config_ref().home_dir.clone();
    assert!(
        !librefang_types::agent_type_store::agent_type_path_in(&home, "user-save").exists(),
        "a refused save must not leave an agent-type file behind"
    );
}

/// `signed_manifest` is verified against the exact bytes it was signed over.
/// A `template` supplies the manifest server-side, so accepting the pair
/// would leave nothing to verify the signature against — the request must be
/// rejected rather than the signature silently ignored.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_rejects_signed_manifest_combined_with_template() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let templates_dir = h.state.kernel.config_ref().home_dir.join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("signed-template.toml"),
        minimal_manifest_toml("signed-template"),
    )
    .unwrap();

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({
                "template": "signed-template",
                "signed_manifest": "{\"not\":\"a signature\"}"
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "signed_manifest + template must be refused: {body}"
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("signed_manifest"),
        "the refusal must name the offending field, not fall through to a \
         different error path: {body}"
    );
}
