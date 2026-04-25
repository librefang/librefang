//! Deterministic fixtures for integration tests.

use librefang_api::routes::AppState;
use librefang_runtime::audit::AuditAction;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Stable agent manifest used by API and workflow integration tests.
pub const FIXTURE_AGENT_MANIFEST: &str = r#"
name = "fixture-agent"
version = "0.1.0"
description = "Deterministic fixture agent"
author = "librefang-tests"
system_prompt = "You are a deterministic integration test fixture."

[model]
provider = "ollama"
model = "test-model"

[runtime]
mode = "semi"
temperature = 0.0
max_tokens = 64
"#;

/// A valid prompt-only SKILL.md fixture.
pub fn fixture_skill_md(name: &str, description: &str) -> String {
    format!(
        r#"---
name: {name}
description: {description}
---

Use deterministic responses. Never call external services during tests.
"#
    )
}

/// An invalid SKILL.md fixture used to assert manifest parsing failures.
pub const INVALID_SKILL_MD: &str =
    "This intentionally lacks YAML frontmatter and must not parse as a skill.";

/// Build an agent manifest that targets a specific provider/model/base URL.
pub fn agent_manifest_for_provider(provider: &str, model: &str, base_url: &str) -> String {
    format!(
        r#"
name = "fixture-agent"
version = "0.1.0"
description = "Deterministic fixture agent"
author = "librefang-tests"
system_prompt = "You are a deterministic integration test fixture."

[model]
provider = "{provider}"
model = "{model}"
base_url = "{base_url}"

[runtime]
mode = "semi"
temperature = 0.0
max_tokens = 64
"#
    )
}

/// Create a local registry skill under `{home}/registry/skills/{name}`.
pub fn seed_registry_skill(home: &Path, name: &str) -> std::io::Result<PathBuf> {
    let skill_dir = home.join("registry").join("skills").join(name);
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        fixture_skill_md(name, "Fixture skill for integration tests"),
    )?;
    Ok(skill_dir)
}

/// Create a malformed registry skill under `{home}/registry/skills/{name}`.
pub fn seed_invalid_registry_skill(home: &Path, name: &str) -> std::io::Result<PathBuf> {
    let skill_dir = home.join("registry").join("skills").join(name);
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("SKILL.md"), INVALID_SKILL_MD)?;
    Ok(skill_dir)
}

/// Create a workflow JSON payload that runs a single deterministic agent step.
pub fn single_step_workflow(name: &str, agent_name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "description": "Integration workflow fixture",
        "steps": [{
            "name": "fixture-step",
            "agent_name": agent_name,
            "prompt": "Echo fixture input: {{input}}",
            "mode": "sequential",
            "timeout_secs": 30
        }]
    })
}

/// Start a tiny OpenAI-compatible fixture server for integration tests.
pub async fn spawn_openai_fixture_server(response_text: &'static str) -> String {
    use axum::routing::{get, post};
    use axum::{Json, Router};

    async fn chat(Json(_body): Json<serde_json::Value>) -> Json<serde_json::Value> {
        Json(json!({
            "id": "chatcmpl-fixture",
            "object": "chat.completion",
            "created": 1,
            "model": "fixture-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "fixture workflow output"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 7,
                "completion_tokens": 3,
                "total_tokens": 10
            }
        }))
    }

    async fn models() -> Json<serde_json::Value> {
        Json(json!({
            "object": "list",
            "data": [{"id": "fixture-model", "object": "model"}]
        }))
    }

    let app = Router::new()
        .route("/chat/completions", post(chat))
        .route("/v1/chat/completions", post(chat))
        .route("/models", get(models))
        .route("/v1/models", get(models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind OpenAI fixture server");
    let addr = listener.local_addr().expect("fixture server local addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("OpenAI fixture server failed");
    });

    let _ = response_text;
    format!("http://{addr}/v1")
}

/// Assert that the in-memory audit log contains an action for a target.
pub fn assert_audit_contains(state: &Arc<AppState>, target: &str, action: AuditAction) {
    let entries = state.kernel.audit().recent(100);
    assert!(
        entries.iter().any(|entry| entry.agent_id == target
            && format!("{:?}", entry.action) == format!("{action:?}")),
        "expected audit log to contain action {action:?} for target {target}; entries: {entries:?}"
    );
}
