//! Integration coverage for #8469: renaming or cloning an agent must carry the new name into the `name:` key of `{workspace}/.identity/IDENTITY.md`.
//!
//! IDENTITY.md is written once at spawn and injected verbatim into the system prompt, so before the fix a renamed agent kept introducing itself by its old name.
//! These tests drive the real routes (`PATCH /api/agents/{id}/config`, `PATCH /api/agents/{id}`, `POST /api/agents/{id}/clone`) through the full production router and read the file back from disk.

use librefang_kernel::LibreFangKernel;
use librefang_types::agent::AgentId;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

struct TestServer {
    base_url: String,
    state: Arc<librefang_api::routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn start_full_router() -> TestServer {
    let tmp = tempfile::tempdir().expect("create temp dir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: String::new(),
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

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel should boot"));
    kernel.set_self_handle();

    let (app, state) = librefang_api::server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        base_url: format!("http://{addr}"),
        state,
        _tmp: tmp,
    }
}

fn manifest(name: &str) -> String {
    format!(
        r#"
name = "{name}"
version = "0.1.0"
description = "Rename identity test agent (#8469)"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."
"#
    )
}

async fn spawn(server: &TestServer, name: &str) -> String {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": manifest(name)}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "spawn must return 201");
    let body: serde_json::Value = resp.json().await.unwrap();
    body["agent_id"].as_str().unwrap().to_string()
}

fn identity_path(server: &TestServer, agent_id: &str) -> PathBuf {
    let id: AgentId = agent_id.parse().expect("agent id");
    let workspace = server
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent in registry")
        .manifest
        .workspace
        .expect("spawned agent has a workspace");
    workspace.join(".identity").join("IDENTITY.md")
}

fn registry_name(server: &TestServer, agent_id: &str) -> String {
    let id: AgentId = agent_id.parse().expect("agent id");
    server
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent in registry")
        .name
}

async fn patch(server: &TestServer, path: &str, body: serde_json::Value) -> reqwest::StatusCode {
    reqwest::Client::new()
        .patch(format!("{}{path}", server.base_url))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_config_rename_updates_identity_front_matter() {
    let server = start_full_router().await;
    let id = spawn(&server, "rename-a").await;
    let path = identity_path(&server, &id);
    let before = std::fs::read_to_string(&path).expect("IDENTITY.md generated at spawn");
    assert!(before.contains("\nname: rename-a\n"), "{before}");

    let status = patch(
        &server,
        &format!("/api/agents/{id}/config"),
        serde_json::json!({"name": "rename-b"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(registry_name(&server, &id), "rename-b");

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before.replacen("\nname: rename-a\n", "\nname: rename-b\n", 1),
        "only the front-matter name changes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_config_rename_keeps_a_deliberate_persona_name() {
    let server = start_full_router().await;
    let id = spawn(&server, "persona-a").await;
    let path = identity_path(&server, &id);
    let custom = std::fs::read_to_string(&path).unwrap().replacen(
        "\nname: persona-a\n",
        "\nname: Jarvis\n",
        1,
    );
    std::fs::write(&path, &custom).unwrap();

    let status = patch(
        &server,
        &format!("/api/agents/{id}/config"),
        serde_json::json!({"name": "persona-b"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(registry_name(&server, &id), "persona-b");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), custom);
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_agent_rename_updates_identity_front_matter() {
    let server = start_full_router().await;
    let id = spawn(&server, "patch-a").await;
    let path = identity_path(&server, &id);
    let before = std::fs::read_to_string(&path).unwrap();

    let status = patch(
        &server,
        &format!("/api/agents/{id}"),
        serde_json::json!({"name": "patch-b"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(registry_name(&server, &id), "patch-b");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before.replacen("\nname: patch-a\n", "\nname: patch-b\n", 1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_rename_leaves_identity_untouched() {
    let server = start_full_router().await;
    let id = spawn(&server, "taken-a").await;
    spawn(&server, "taken-b").await;
    let path = identity_path(&server, &id);
    let before = std::fs::read_to_string(&path).unwrap();

    let status = patch(
        &server,
        &format!("/api/agents/{id}/config"),
        serde_json::json!({"name": "taken-b"}),
    )
    .await;
    assert_eq!(status, 409, "duplicate name keeps the existing 409 mapping");
    assert_eq!(registry_name(&server, &id), "taken-a");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn clone_points_copied_identity_at_the_clone_name() {
    let server = start_full_router().await;
    let source = spawn(&server, "clone-src").await;
    let source_identity = std::fs::read_to_string(identity_path(&server, &source)).unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents/{source}/clone", server.base_url))
        .json(&serde_json::json!({"new_name": "clone-dst"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let clone = body["agent_id"]
        .as_str()
        .expect("clone agent_id")
        .to_string();

    assert_eq!(
        std::fs::read_to_string(identity_path(&server, &clone)).unwrap(),
        source_identity.replacen("\nname: clone-src\n", "\nname: clone-dst\n", 1),
        "the copied IDENTITY.md must name the clone, not the source"
    );
    assert_eq!(
        std::fs::read_to_string(identity_path(&server, &source)).unwrap(),
        source_identity,
        "the source's own file is untouched"
    );
}
