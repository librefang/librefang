//! Integration tests for the "Agent Types page is uneditable" bug (an install
//! with 42 agents and 0 templates had every agent listed as an unusable agent
//! type):
//!
//!  1. `POST /api/agents {"template": name}` resolves templates through the kernel-owned `agent_template_candidates` order: the writable `agent-types/<name>.toml` store first, then the live agent's `workspaces/agents/<name>/agent.toml`, then the read-only registry checkout.
//!     This file pins the route's end of that contract, including the refusal of `signed_manifest` combined with `template`.
//!  2. `POST /api/agents/{id}/save-as-agent-type` extracts a live agent's manifest into a reusable `agent-types/<name>.toml` file — the bridge from "I have agents, not templates" to a populated, editable Agent Types page.
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
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, ModelConfig, SessionId,
};
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
// POST /api/agents {"template": name} resolves real templates
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

    // The writable `agent-types/` store is the first candidate in the kernel-owned order, so this resolves even with no workspace agent of that name.
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

    // Backward-compat fallback: a
    // "template" name that matches an existing agent's own workspace
    // directory still resolves. It is the second candidate in the kernel's
    // `agent_template_candidates` order — `agent-types/` → `workspaces/agents/`
    // → `registry/agents/` — not "behind templates/", which is a different
    // (registry checkout) store.
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

/// A workspace agent whose declared name is not sanitized must still resolve.
///
/// A live agent named `Research Bot` lives at
/// `workspaces/agents/ResearchBot/agent.toml` — the directory is the sanitized
/// component of its name (pinned by `workspace_component("Research Bot") ==
/// "ResearchBot"` in the kernel). `resolve_manifest` does not pin the
/// template's declared `name` to the requested one, so the request matches on
/// the directory component and `Research Bot 2` (the request's own name
/// override) is what spawns.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_from_workspace_agent_with_space_in_declared_name() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let agent_dir = h
        .state
        .kernel
        .config_ref()
        .home_dir
        .join("workspaces")
        .join("agents")
        .join("ResearchBot");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("agent.toml"),
        minimal_manifest_toml("Research Bot"),
    )
    .unwrap();

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({"template": "ResearchBot", "name": "Research Bot 2"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a workspace agent with a space in its declared name must still spawn: {body}"
    );

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
    assert_eq!(entry.manifest.name, "Research Bot 2");
    assert_eq!(
        entry.manifest.source_template.as_deref(),
        Some("ResearchBot"),
        "the resolved template name must be recorded even for a name-mismatched file"
    );
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
// POST /api/agents/{id}/save-as-agent-type + round trip
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
        // Paused source: `enabled` is instance-local state (like `workspace`)
        // and must not be frozen into the saved type — see the assertion on
        // the saved TOML below.
        enabled: false,
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
    // The source was paused; the saved type must not be. `enabled` is
    // instance-local state (`manifest_privacy.rs` classifies it `LocalState`),
    // so it is read from the raw TOML: an omitted key would parse as the
    // default `true` and make an assertion on the struct pass vacuously.
    let saved_raw: toml::Value = toml::from_str(&content).unwrap();
    assert_eq!(
        saved_raw.get("enabled").and_then(|v| v.as_bool()),
        Some(true),
        "a paused source must not save a paused type: {content}"
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

    // It shows up on the Agent Types list, editable/deletable like any other template.
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
    assert!(
        saved_raw.get("workspace").is_none(),
        "the saved template must not carry a workspace key: {content}"
    );
    assert!(
        new_entry.manifest.workspace.is_some(),
        "the spawned agent must still get its own workspace from spawn"
    );
}

/// A hand-spawned agent's live manifest is refused as a template source.
///
/// Hand activation sets `manifest.is_hand = true` (`kernel/hands_lifecycle.rs`),
/// and a `#[serde(default)]` field like that would be persisted verbatim into
/// `agent-types/<name>.toml`. On restart the type's agents then load with
/// `entry.is_hand = true` (`librefang-memory/src/structured.rs`), boot skips
/// the row and the orphaned-hand GC deletes it together with its sessions.
/// The hand's injected `capabilities.tools` / skills merge would travel with
/// it too. The delete paths refuse hand-spawned agents the same way
/// (`hand_agent_delete_denied`).
#[tokio::test(flavor = "multi_thread")]
async fn save_agent_as_agent_type_refuses_hand_sources() {
    let _guard = home_lock().lock().await;
    let h = boot().await;

    let mut source = AgentManifest {
        name: "hand-worker".to_string(),
        tags: vec!["hand:researcher".to_string(), "hand_role:lead".to_string()],
        ..AgentManifest::default()
    };
    source.is_hand = true;
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(source)
        .expect("spawn_agent");
    assert!(
        h.state
            .kernel
            .agent_registry()
            .get(agent_id)
            .expect("source entry")
            .is_hand,
        "precondition: spawn must derive `is_hand` from the hand tags"
    );

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{agent_id}/save-as-agent-type"),
            serde_json::json!({"template_name": "snapshotted-hand"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a hand-spawned source must be refused: {body}"
    );
    assert_eq!(
        body["code"].as_str(),
        Some("hand_agent_save_denied"),
        "{body}"
    );

    let templates_dir = h.state.kernel.config_ref().home_dir.join("agent-types");
    assert!(
        !templates_dir.join("snapshotted-hand.toml").exists(),
        "a refused save must not leave an agent-type file behind"
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
    // The flag the `entry.is_hand` guard above cannot see; it makes the
    // `!saved.is_hand` assertion below non-vacuous — without the snapshot's
    // reset, the saved TOML would carry it back.
    source.is_hand = true;
    // Registered directly rather than spawned: `spawn` derives
    // `entry.is_hand` from `hand:*` tags, and the handler refuses
    // `entry.is_hand` sources outright (see the test above). This entry
    // models the case that guard cannot see: a manifest carrying hand-ness
    // in its own fields — `is_hand = true` (set below) plus `hand:*` tags —
    // reloaded from an edited `agent.toml` at startup, where the flag is not
    // recomputed from the tags and the registered `entry.is_hand` stays
    // `false` (the `AgentEntry` default). The snapshot's scrub is what has to
    // strip both, or a type saved from it would reload with
    // `entry.is_hand = true`, and its agents would be skipped at boot and
    // reclaimed by the orphaned-hand GC.
    let agent_id = AgentId::new();
    h.state
        .kernel
        .agent_registry()
        .register(AgentEntry {
            id: agent_id,
            name: source.name.clone(),
            manifest: source,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            session_id: SessionId::new(),
            ..Default::default()
        })
        .expect("register source entry");

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
    assert!(
        !saved.is_hand,
        "the snapshot must never carry `is_hand`: {content}"
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
    assert_eq!(
        body["code"].as_str(),
        Some("template_exists"),
        "a name collision on disk must carry its own code, not the shadow one: {body}"
    );
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
/// subsequent catalog read and leaves the agent unreachable. Since template
/// resolution consults the agent-type store first, a shadowing type wins
/// outright for `POST /api/agents {"template": name}` as well.
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
    assert_eq!(
        body["code"].as_str(),
        Some("template_name_taken"),
        "the shadow refusal must be distinguishable from a plain name collision: {body}"
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
/// `template` supplies the manifest server-side, so pairing the two leaves no client-supplied bytes to verify the signature against.
/// `POST /api/agents` rejects the pair with `400 invalid_manifest` (`api-error-manifest-signed-template-conflict`).
/// A signature-pinned spawn therefore pins only content the client supplied, never whatever the template store holds at verification time.
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
