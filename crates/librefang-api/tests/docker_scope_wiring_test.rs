//! `[docker] scope` must actually govern Docker container lifetime, not just round-trip
//! through `GET /api/config`.
//!
//! Before the container pool was wired up, `scope`, `reuse_cool_secs`, `idle_timeout_secs`
//! and `max_age_secs` were read back verbatim by the config API while no execution path
//! consumed any of them: every `docker_exec` call created a container and destroyed it again,
//! so all three scope values behaved identically. These tests walk the surface an operator
//! actually touches — boot with a `[docker]` table, read it back over HTTP — and then feed the
//! kernel's live config into the pool-key derivation and the pool itself, so a regression that
//! leaves the keys inert again fails here rather than silently.
//!
//! What lives elsewhere: the reaper that ends a pooled container's life is asserted against
//! kernel shutdown in `librefang-kernel/tests/docker_pool_reaper_test.rs`, and the acquire /
//! release predicate has its own unit tests in `librefang-runtime-sandbox-docker`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_runtime::docker_sandbox::{ContainerPool, PoolKey, PoolOwner, SandboxContainer};
use librefang_types::config::{DefaultModelConfig, DockerScope, KernelConfig};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

/// A container that never existed on any Docker daemon.
///
/// The pool's acquire / release path is a pure map operation — only `cleanup` and `drain` shell
/// out to `docker` — so a synthetic container exercises reuse on a host with no daemon. Each
/// test uses its own [`ContainerPool`] rather than `docker_sandbox::global_pool`, which is
/// process-wide and would let these tests see each other's containers.
fn synthetic_container(id: &str, agent: &str) -> SandboxContainer {
    SandboxContainer {
        container_id: id.to_string(),
        agent_id: agent.to_string(),
        created_at: chrono::Utc::now(),
    }
}

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot_router(customize: impl FnOnce(&mut KernelConfig)) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
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
    customize(&mut config);

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        _tmp: tmp,
        state,
    }
}

async fn get_config_json(h: &RouterHarness) -> serde_json::Value {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/config")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).expect("config response is JSON")
}

/// `scope = "agent"` is what the operator set, what `GET /api/config` reports, and — the part
/// that used to be missing — what decides whether two sessions of one agent share a container.
#[tokio::test(flavor = "multi_thread")]
async fn agent_scope_from_config_pools_one_container_across_sessions() {
    let h = boot_router(|c| {
        c.docker.enabled = true;
        c.docker.scope = DockerScope::Agent;
        c.docker.idle_timeout_secs = 900;
        c.docker.max_age_secs = 7200;
    })
    .await;

    let json = get_config_json(&h).await;
    assert_eq!(json["docker"]["scope"], serde_json::json!("agent"));
    assert_eq!(json["docker"]["idle_timeout_secs"], serde_json::json!(900));
    assert_eq!(json["docker"]["max_age_secs"], serde_json::json!(7200));

    let docker = h.state.kernel.config_ref().docker.clone();
    let workspace = Path::new("/workspaces/researcher");
    let in_session_a = PoolKey::derive(&docker, "researcher", Some("session-a"), workspace)
        .expect("agent scope always derives a key");
    let in_session_b = PoolKey::derive(&docker, "researcher", Some("session-b"), workspace)
        .expect("agent scope always derives a key");

    assert_eq!(
        in_session_a, in_session_b,
        "scope = agent must reuse one container across the agent's sessions"
    );
    assert_eq!(
        in_session_a.owner,
        PoolOwner::Agent {
            agent_id: "researcher".to_string()
        }
    );

    // A different agent still gets its own container.
    let other_agent = PoolKey::derive(
        &docker,
        "editor",
        Some("session-a"),
        Path::new("/workspaces/editor"),
    )
    .expect("agent scope always derives a key");
    assert_ne!(in_session_a, other_agent);

    // Drive the pool the keys are for: session A's container must come back out for session B,
    // and must not be handed to a different agent. `reuse_cool_secs` is passed at its configured
    // default because an agent-scoped container is never subject to it.
    let pool = ContainerPool::new();
    pool.release(
        synthetic_container("agent-scope-1", "researcher"),
        in_session_a,
        "researcher",
    );
    assert!(
        pool.acquire(&other_agent, "editor", docker.reuse_cool_secs)
            .is_none(),
        "scope = agent must not hand one agent's container to another"
    );
    assert_eq!(
        pool.acquire(&in_session_b, "researcher", docker.reuse_cool_secs)
            .map(|c| c.container_id),
        Some("agent-scope-1".to_string()),
        "scope = agent must reuse the container across the agent's sessions"
    );
    assert!(pool.is_empty());
}

/// `scope = "session"` — the default — must separate sessions of the same agent. With the keys
/// inert this was indistinguishable from `agent` and from `shared`.
#[tokio::test(flavor = "multi_thread")]
async fn session_scope_from_config_separates_sessions_of_one_agent() {
    let h = boot_router(|c| {
        c.docker.enabled = true;
    })
    .await;

    let json = get_config_json(&h).await;
    assert_eq!(
        json["docker"]["scope"],
        serde_json::json!("session"),
        "session is the documented default"
    );

    let docker = h.state.kernel.config_ref().docker.clone();
    let workspace = Path::new("/workspaces/researcher");
    let a = PoolKey::derive(&docker, "researcher", Some("session-a"), workspace)
        .expect("a session-scoped call with a session derives a key");
    let b = PoolKey::derive(&docker, "researcher", Some("session-b"), workspace)
        .expect("a session-scoped call with a session derives a key");
    assert_ne!(
        a, b,
        "scope = session must not hand one session's container to another"
    );

    // The REST tool bridge has no session to pin a container to, so such a call stays
    // create-exec-destroy instead of quietly widening to an agent-scoped container.
    assert!(PoolKey::derive(&docker, "researcher", None, workspace).is_none());
}

/// `scope = "shared"` pools across agents, but only across agents that mount the same
/// workspace — the container carries the workspace it was created with.
#[tokio::test(flavor = "multi_thread")]
async fn shared_scope_from_config_pools_across_agents_but_not_across_workspaces() {
    let h = boot_router(|c| {
        c.docker.enabled = true;
        c.docker.scope = DockerScope::Shared;
    })
    .await;

    let json = get_config_json(&h).await;
    assert_eq!(json["docker"]["scope"], serde_json::json!("shared"));

    let docker = h.state.kernel.config_ref().docker.clone();
    let shared_ws = Path::new("/workspaces/team");
    let first = PoolKey::derive(&docker, "researcher", Some("session-a"), shared_ws)
        .expect("shared scope always derives a key");
    let second = PoolKey::derive(&docker, "editor", Some("session-b"), shared_ws)
        .expect("shared scope always derives a key");
    assert_eq!(first, second, "scope = shared pools across agents");
    assert_eq!(first.owner, PoolOwner::Shared);

    let elsewhere = PoolKey::derive(
        &docker,
        "editor",
        Some("session-b"),
        Path::new("/workspaces/editor"),
    )
    .expect("shared scope always derives a key");
    assert_ne!(
        first, elsewhere,
        "a shared container must not be handed to a caller mounting a different workspace"
    );

    // `reuse_cool_secs` at its configured default (300) delays a handover between two agents,
    // and only that. The agent that released the container picks it straight back up — without
    // that, every acquire under `shared` would be blocked, and since a blocked acquire creates
    // another container instead of waiting, the pool would grow rather than converge on one.
    let pool = ContainerPool::new();
    pool.release(
        synthetic_container("shared-scope-1", "researcher"),
        first.clone(),
        "researcher",
    );
    assert!(
        pool.acquire(&second, "editor", docker.reuse_cool_secs)
            .is_none(),
        "a container another agent just used must settle before it is handed over"
    );
    assert_eq!(
        pool.acquire(&first, "researcher", docker.reuse_cool_secs)
            .map(|c| c.container_id),
        Some("shared-scope-1".to_string()),
        "the releasing agent must reuse its own shared container immediately"
    );
    assert!(pool.is_empty());
}
