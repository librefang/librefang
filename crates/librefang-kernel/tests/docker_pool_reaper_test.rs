// `start_background_agents()` folds every spawned closure's async-block layout into a single
// type-resolution query, which overruns the default recursion limit of 128 — same reason
// `audit_retention_test.rs` raises it.
#![recursion_limit = "256"]

//! The Docker container reaper must be wired into the kernel lifecycle, not merely defined.
//!
//! `[docker] scope` lets a sandbox container outlive the tool call that created it, which makes
//! the reaper the only thing that ever ends that life: it applies `idle_timeout_secs` /
//! `max_age_secs` on a 60s tick and destroys every pooled container when the daemon stops.
//! `entry_is_stale` and `PoolKey::derive` are unit-tested inside
//! `librefang-runtime-sandbox-docker`, but a pure function nobody calls reaps nothing — deleting
//! the `if cfg.docker.enabled { … }` block from `start_background_agents` would leave all of
//! those green while returning both timeouts to the inert state they were in before the pool was
//! wired up. These tests therefore assert against the observable effect of the loop existing:
//! kernel shutdown empties the process-wide pool when the sandbox is enabled, and leaves it
//! untouched when it is not.
//!
//! Both phases live in one test on purpose. `docker_sandbox::global_pool()` is process-wide, so
//! two tests in this binary running concurrently would each see the other's containers — and the
//! drain phase would empty the pool the "nothing is scheduled" phase is asserting about.

use librefang_runtime::docker_sandbox::{global_pool, PoolKey, SandboxContainer};
use librefang_testing::MockKernelBuilder;
use librefang_types::config::{DockerSandboxConfig, DockerScope};
use std::path::Path;
use std::time::{Duration, Instant};

const AGENT: &str = "reaper-agent";
const WORKSPACE: &str = "/workspaces/reaper-agent";

/// A container that never existed on any Docker daemon. Nothing in this test execs into it; the
/// reaper's `destroy_sandbox` shells out to `docker rm -f`, which fails harmlessly (no daemon in
/// CI, unknown id on a developer box) and is ignored either way — the pool entry is dropped
/// regardless, which is the state being asserted on.
fn synthetic_container(id: &str) -> SandboxContainer {
    SandboxContainer {
        container_id: id.to_string(),
        agent_id: AGENT.to_string(),
        created_at: chrono::Utc::now(),
    }
}

fn pool_key(enabled: bool) -> PoolKey {
    let config = DockerSandboxConfig {
        enabled,
        scope: DockerScope::Agent,
        ..Default::default()
    };
    PoolKey::derive(&config, AGENT, None, Path::new(WORKSPACE))
        .expect("agent scope always derives a key")
}

// `start_background_agents` reaches kernel paths that call `tokio::task::block_in_place`, which
// panics on the default current-thread runtime.
#[tokio::test(flavor = "multi_thread")]
async fn docker_pool_reaper_is_wired_to_kernel_shutdown() {
    let pool = global_pool();

    // ── Phase 1: `docker.enabled = false` schedules nothing. ──────────────────────────────
    let disabled_key = pool_key(false);
    pool.release(
        synthetic_container("reaper-wiring-disabled"),
        disabled_key.clone(),
        AGENT,
    );

    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.docker.enabled = false;
            c.docker.scope = DockerScope::Agent;
        })
        .build();
    kernel.start_background_agents().await;
    kernel.shutdown();
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(
        pool.acquire(&disabled_key, AGENT, 0)
            .map(|c| c.container_id),
        Some("reaper-wiring-disabled".to_string()),
        "with docker.enabled = false nothing is ever pooled by the daemon, so no reaper should \
         be scheduled and the pool must be left exactly as it was"
    );
    assert!(pool.is_empty(), "phase 1 must hand the pool back empty");
    drop(kernel);

    // ── Phase 2: `docker.enabled = true` drains the pool at stop time. ────────────────────
    let enabled_key = pool_key(true);
    pool.release(
        synthetic_container("reaper-wiring-enabled"),
        enabled_key.clone(),
        AGENT,
    );
    assert_eq!(pool.len(), 1);

    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.docker.enabled = true;
            c.docker.scope = DockerScope::Agent;
            c.docker.idle_timeout_secs = 1;
            c.docker.max_age_secs = 1;
        })
        .build();
    kernel.start_background_agents().await;

    // Shutdown is the reaper's other exit: it races the 60s tick against the supervisor's
    // shutdown watch precisely so the drain happens at stop time rather than up to a minute
    // later, when the process is already gone and the containers are stranded.
    kernel.shutdown();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !pool.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        pool.is_empty(),
        "the reaper must be spawned when the sandbox is enabled and must destroy every pooled \
         container on kernel shutdown; {} left behind",
        pool.len()
    );
}
