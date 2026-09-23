//! Integration coverage for #8447: an agent's identity has two owners, and each identity field is written to the one that reads it.
//!
//! Appearance (`emoji`, `avatar_url`, `color`) belongs to the registry, which is what the dashboard draws avatars from.
//! Personality (`archetype`, `vibe`, `greeting_style`) belongs to the front matter of `{workspace}/.identity/IDENTITY.md`, which is what reaches the prompt.
//! Before the fix both PATCH routes stored all six in the registry, so editing an agent's personality answered 200 and changed nothing the agent could see.
//! These tests drive the real routes through the full production router and read the file back from disk.

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
description = "Identity ownership test agent (#8447)"
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

async fn patch(server: &TestServer, path: &str, body: serde_json::Value) -> reqwest::StatusCode {
    reqwest::Client::new()
        .patch(format!("{}{path}", server.base_url))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
}

async fn get_agent(server: &TestServer, agent_id: &str) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .get(format!("{}/api/agents/{agent_id}", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

/// Give the spawn-time file a body an edit could plausibly disturb: prose that mentions the same keys the front matter carries.
fn seed_identity_with_body(path: &PathBuf) -> String {
    let mut content = std::fs::read_to_string(path).expect("IDENTITY.md generated at spawn");
    content.push_str("\nOperator notes.\nvibe: this line is prose, not front matter\n");
    std::fs::write(path, &content).unwrap();
    content
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_identity_personality_rewrites_front_matter_and_keeps_the_body() {
    let server = start_full_router().await;
    let id = spawn(&server, "personality-identity").await;
    let path = identity_path(&server, &id);
    let before = seed_identity_with_body(&path);
    assert!(before.contains("\narchetype: assistant\n"), "{before}");

    let status = patch(
        &server,
        &format!("/api/agents/{id}/identity"),
        serde_json::json!({"archetype": "researcher", "vibe": "technical", "greeting_style": "brief"}),
    )
    .await;
    assert_eq!(status, 200);

    let expected = before
        .replacen("\narchetype: assistant\n", "\narchetype: researcher\n", 1)
        .replacen("\nvibe: helpful\n", "\nvibe: technical\n", 1)
        .replacen("\ngreeting_style: warm\n", "\ngreeting_style: brief\n", 1);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        expected,
        "only the three front-matter keys change; the body, including its own `vibe:` line, is untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_config_personality_adds_a_missing_key_inside_the_front_matter() {
    let server = start_full_router().await;
    let id = spawn(&server, "personality-config").await;
    let path = identity_path(&server, &id);
    let without_key = seed_identity_with_body(&path).replacen("greeting_style: warm\n", "", 1);
    std::fs::write(&path, &without_key).unwrap();

    let status = patch(
        &server,
        &format!("/api/agents/{id}/config"),
        serde_json::json!({"greeting_style": "formal"}),
    )
    .await;
    assert_eq!(status, 200);

    let after = std::fs::read_to_string(&path).unwrap();
    let (front_matter, body) = after
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .expect("the file still opens with a closed front-matter block");
    assert!(
        front_matter.ends_with("\ngreeting_style: formal"),
        "the missing key is added inside the block, before its closing fence: {after}"
    );
    let (_, body_before) = without_key
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .unwrap();
    assert_eq!(body, body_before, "the body is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_appearance_leaves_identity_md_untouched() {
    let server = start_full_router().await;
    let id = spawn(&server, "appearance-only").await;
    let path = identity_path(&server, &id);
    let before = std::fs::read_to_string(&path).unwrap();

    for route in ["identity", "config"] {
        let status = patch(
            &server,
            &format!("/api/agents/{id}/{route}"),
            serde_json::json!({"emoji": "🦊", "avatar_url": "https://example.invalid/a.png", "color": "#123456"}),
        )
        .await;
        assert_eq!(status, 200, "PATCH /{route}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "appearance is owned by the registry; PATCH /{route} must not touch IDENTITY.md"
        );
    }

    let body = get_agent(&server, &id).await;
    assert_eq!(body["identity"]["emoji"], serde_json::json!("🦊"));
    assert_eq!(body["identity"]["color"], serde_json::json!("#123456"));
}

#[tokio::test(flavor = "multi_thread")]
async fn personality_value_with_a_line_break_is_rejected_before_anything_changes() {
    let server = start_full_router().await;
    let id = spawn(&server, "personality-newline").await;
    let path = identity_path(&server, &id);
    let before = std::fs::read_to_string(&path).unwrap();

    let status = patch(
        &server,
        &format!("/api/agents/{id}/identity"),
        serde_json::json!({"emoji": "🐙", "vibe": "calm\nname: someone-else"}),
    )
    .await;
    assert_eq!(
        status, 400,
        "a line break would inject a second front-matter key"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    let body = get_agent(&server, &id).await;
    assert_ne!(
        body["identity"]["emoji"],
        serde_json::json!("🐙"),
        "a rejected body applies none of its fields"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unterminated_front_matter_is_refused_not_guessed_at() {
    let server = start_full_router().await;
    let id = spawn(&server, "personality-unterminated").await;
    let path = identity_path(&server, &id);
    let broken = "---\nname: personality-unterminated\nvibe: helpful\n# Identity\n";
    std::fs::write(&path, broken).unwrap();

    let status = patch(
        &server,
        &format!("/api/agents/{id}/identity"),
        serde_json::json!({"vibe": "technical"}),
    )
    .await;
    assert_eq!(
        status, 409,
        "there is no block whose end is known, so the edit cannot be placed without guessing"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
}
