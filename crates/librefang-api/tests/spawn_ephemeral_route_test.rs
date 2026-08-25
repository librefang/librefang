//! Integration tests for `POST /api/agents/spawn-ephemeral` (#6699).
//!
//! The ephemeral worker engine (#7875) landed reachable only from inside a running agent's turn, through `agent_spawn`'s `ephemeral: true` branch.
//! This route is its HTTP entry point, and these tests exercise it through the production router (`server::build_router` + `tower::ServiceExt::oneshot`), so a handler that exists but was never merged into `server.rs` fails here rather than shipping.
//!
//! ## No test reaches an LLM
//!
//! Every assertion below lands on a refusal the engine makes *before* it builds the driver: the parent lookup, the agent-type resolution, and the tool-set narrowing all run ahead of the mission workspace and the model call.
//! That ordering is load-bearing for this file — a test that got as far as the driver would either hang on `message_timeout_secs` or assert on whatever a missing Ollama returns.
//!
//! ## What `spawns_from_a_type_authored_in_the_agent_type_store` is for
//!
//! `POST /api/templates` and the `agent_type_create` tool write to `$HOME/agent-types/<name>.toml`, and until #6699 the agent-type resolver looked only in `workspaces/agents/<name>/agent.toml` and `registry/agents/<name>/agent.toml`.
//! So every type the dashboard could create was invisible to the one engine whose job is to run it, and the dashboard's Quick Run would have failed on the entire catalog it renders.
//! The test pins the flat store onto the search path; `rejects_an_unknown_agent_type` is its control, proving the assertion can fail.
//!
//! Run: cargo test -p librefang-api --test spawn_ephemeral_route_test

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
    home: std::path::PathBuf,
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

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let home = kernel.home_dir().to_path_buf();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        home,
        _tmp: tmp,
    }
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

/// Write one agent type into the flat store the dashboard and `agent_type_create` write to.
fn write_agent_type(home: &std::path::Path, name: &str, body: &str) {
    let dir = home.join("agent-types");
    std::fs::create_dir_all(&dir).expect("agent-types dir");
    std::fs::write(dir.join(format!("{name}.toml")), body).expect("write agent type");
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

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::String(
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
    };
    (status, json)
}

/// The whole response body as one lowercase string, so an assertion can look for a phrase without caring which of `message` / `error.message` / `detail` carried it.
fn rendered(body: &serde_json::Value) -> String {
    body.to_string().to_lowercase()
}

/// A tool name no builtin, skill, MCP server or plugin will ever answer to.
///
/// Requesting it forces the engine to refuse during tool-set narrowing — after the agent type has been resolved and before any driver is built — which is what lets these tests assert on template resolution without an LLM.
const NO_SUCH_TOOL: &str = "definitely_not_a_real_tool_6699";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The route is registered and reachable, and an unknown parent is a JSON 404 rather than axum's `text/plain` fallback.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_parent_is_a_json_404() {
    let h = boot().await;

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/spawn-ephemeral",
            serde_json::json!({ "parent": "nobody-by-that-name", "message": "hi" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body={body:?}");
    assert!(body.is_object(), "a handler 404 must be JSON, got {body:?}");
    assert_eq!(
        body["error"]["code"], "agent_not_found",
        "the refusal must carry a machine-readable code, got {body:?}"
    );
}

/// A parent named rather than addressed by UUID resolves, because that is how the dashboard will call this: it renders agent names, not ids.
///
/// Proven by the refusal that comes *after* the lookup — a tool the parent cannot call — rather than by a successful run, which would need a model.
#[tokio::test(flavor = "multi_thread")]
async fn a_parent_may_be_addressed_by_name() {
    let h = boot().await;
    spawn_named(&h.state, "eph-parent-by-name");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/spawn-ephemeral",
            serde_json::json!({
                "parent": "eph-parent-by-name",
                "message": "hi",
                "tools": [NO_SUCH_TOOL],
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    let text = rendered(&body);
    assert!(
        text.contains(NO_SUCH_TOOL),
        "the refusal must name the offending tool rather than dropping it silently, got {body:?}"
    );
}

/// A tool the parent cannot itself call is refused by name.
///
/// The engine's contract is that the advertised set equals the executable set, and that a typo is never indistinguishable from success — this is that contract seen from the HTTP surface.
#[tokio::test(flavor = "multi_thread")]
async fn an_untouchable_tool_is_refused_by_name() {
    let h = boot().await;
    let id = spawn_named(&h.state, "eph-tool-guard");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/spawn-ephemeral",
            serde_json::json!({
                "parent": id.to_string(),
                "message": "hi",
                "tools": [NO_SUCH_TOOL],
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert!(rendered(&body).contains(NO_SUCH_TOOL), "body={body:?}");
}

/// The control for the test below: a type that exists nowhere is reported as missing, and the message names the paths searched.
#[tokio::test(flavor = "multi_thread")]
async fn rejects_an_unknown_agent_type() {
    let h = boot().await;
    let id = spawn_named(&h.state, "eph-unknown-type");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/spawn-ephemeral",
            serde_json::json!({
                "parent": id.to_string(),
                "message": "hi",
                "agent_type": "no-such-type",
                "tools": [NO_SUCH_TOOL],
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    let text = rendered(&body);
    assert!(
        text.contains("no template named"),
        "an absent type must be reported as missing, got {body:?}"
    );
    assert!(
        text.contains("agent-types"),
        "the message must name the writable store among the paths searched, got {body:?}"
    );
}

/// The gap this issue's last piece closes: a type authored through `POST /api/templates` or `agent_type_create` lands in the flat `agent-types/` store, which the agent-type resolver did not search.
///
/// Getting past template resolution — into the tool-set refusal, which happens strictly after it — is the observable proof the store is on the search path.
#[tokio::test(flavor = "multi_thread")]
async fn spawns_from_a_type_authored_in_the_agent_type_store() {
    let h = boot().await;
    let id = spawn_named(&h.state, "eph-store-type");
    write_agent_type(
        &h.home,
        "dashboard-researcher",
        "name = \"dashboard-researcher\"\ndescription = \"authored through the dashboard\"\n",
    );

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/spawn-ephemeral",
            serde_json::json!({
                "parent": id.to_string(),
                "message": "hi",
                "agent_type": "dashboard-researcher",
                "tools": [NO_SUCH_TOOL],
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    let text = rendered(&body);
    assert!(
        !text.contains("no template named"),
        "an agent type in the writable store must resolve, got {body:?}"
    );
    assert!(
        text.contains(NO_SUCH_TOOL),
        "resolution should have proceeded to the tool-set check, got {body:?}"
    );
}

/// A body missing `message` is rejected by the extractor, not by a panic or a run with an empty task.
#[tokio::test(flavor = "multi_thread")]
async fn a_body_without_a_message_is_rejected() {
    let h = boot().await;
    let id = spawn_named(&h.state, "eph-no-message");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/spawn-ephemeral",
            serde_json::json!({ "parent": id.to_string() }),
        ),
    )
    .await;

    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "expected a 4xx from the extractor, got {status} body={body:?}"
    );
}
