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
//! (`GET/POST/PUT/DELETE /api/agent-types`) resolve `agent-types/` through
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
use librefang_types::config::{DefaultModelConfig, KernelConfig};
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
    let tmp = tempfile::tempdir().expect("tempdir");
    // Safety: env mutation, serialised by `home_lock()`.
    std::env::set_var("LIBREFANG_HOME", tmp.path());

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

    // Same name in both places — `templates/` must win, matching the
    // precedence `resolve_ephemeral_manifest` (messaging.rs) and
    // `load_agent_manifest_from_template_dirs` (spawn.rs) already use.
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

    // The new agent must NOT share the source agent's workspace directory.
    let source_entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("source entry");
    assert_ne!(
        new_entry.manifest.workspace, source_entry.manifest.workspace,
        "cloned-via-template agent must get its own workspace, not the source's"
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
