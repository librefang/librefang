//! Integration coverage for the dynamic command catalog.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    test: TestAppState,
}

fn install_skill(state: &AppState, root: &std::path::Path, name: &str, description: &str) {
    let skill_dir = root.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        format!(
            r#"
[skill]
name = "{name}"
version = "0.1.0"
description = "{description}"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "Test tool"
input_schema = {{ type = "object" }}
"#
        ),
    )
    .unwrap();
    std::fs::write(skill_dir.join("main.py"), "def main():\n    return None\n").unwrap();
    state
        .kernel
        .skill_registry_ref()
        .write()
        .unwrap()
        .load_skill(&skill_dir)
        .unwrap();
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::commands::router())
        .with_state(state.clone());
    Harness { app, state, test }
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn builtins_win_collisions_and_skill_commands_remain_queryable() {
    let harness = boot().await;
    let skills_dir = harness.test.tmp_path().join("command-skills");
    install_skill(
        &harness.state,
        &skills_dir,
        "HeLp",
        "A skill must not replace builtin help",
    );
    install_skill(
        &harness.state,
        &skills_dir,
        "weather",
        "Show the current weather",
    );

    let (status, body) = get_json(&harness.app, "/api/commands").await;
    assert_eq!(status, StatusCode::OK);
    let commands = body["commands"].as_array().unwrap();
    let help: Vec<_> = commands
        .iter()
        .filter(|entry| {
            entry["cmd"]
                .as_str()
                .is_some_and(|command| command.eq_ignore_ascii_case("/help"))
        })
        .collect();
    assert_eq!(help.len(), 1);
    assert!(help[0].get("source").is_none());
    let weather = commands
        .iter()
        .find(|entry| entry["cmd"] == "/weather")
        .unwrap();
    assert_eq!(weather["source"], "skill");
    assert_eq!(weather["desc"], "Show the current weather");

    let (status, help) = get_json(&harness.app, "/api/commands/HELP").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(help["cmd"], "/help");
    assert!(help.get("source").is_none());

    let (status, weather) = get_json(&harness.app, "/api/commands/weather").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(weather["cmd"], "/weather");
    assert_eq!(weather["source"], "skill");
}
