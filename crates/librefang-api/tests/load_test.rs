//! Load & performance tests for the LibreFang API.
//!
//! Measures throughput under concurrent access: agent spawning, API endpoint
//! latency, session management, and memory usage.
//!
//! These tests drive the **full production router** (`server::build_router`)
//! over a real TCP socket, so every request crosses the same middleware stack
//! the daemon serves: auth, GCRA + auth-login rate-limiting, idempotency,
//! JSON-depth / body-size guards, security headers, and API-version headers.
//! Previously they used a hand-rolled mock router that bypassed all of those
//! layers (refs `docs/issues/integration-tests-mock-router.md`).
//!
//! ### Rate-limiting under load
//!
//! The full router applies real per-IP rate limiting. These tests bind the
//! server to `127.0.0.1` and inject the peer `SocketAddr` via
//! `into_make_service_with_connect_info`, exactly as the daemon does. Both the
//! GCRA limiter and the auth-login limiter exempt loopback callers that send no
//! forwarding header (`rate_limiter.rs`: "Loopback (127.0.0.0/8 + ::1) bypasses
//! the limiter"), which is the documented production behavior for a same-host
//! caller (the dashboard, the CLI, a local agent). The load clients here are
//! exactly that — plain reqwest calls from loopback with no `X-Forwarded-For` —
//! so the limiter *runs and evaluates the loopback-bypass branch on every
//! request* without 429'ing the burst. This preserves each test's
//! throughput / concurrency intent while still exercising the real middleware
//! wiring; we deliberately do NOT disable any layer or weaken any assertion.
//!
//! Auth is configured open (empty `api_key`); combined with the genuine
//! loopback peer the auth middleware lets requests through without a bearer
//! token (`middleware.rs`: empty key + loopback ConnectInfo => pass).
//!
//! Run: cargo test -p librefang-api --test load_test -- --nocapture

use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Race-hardening helpers (#3817)
// ---------------------------------------------------------------------------
//
// `load_concurrent_agent_spawns` and `load_spawn_kill_cycle` exercise the
// kernel's concurrent agent lifecycle through the HTTP layer. The underlying
// register/remove publish-order race in `AgentRegistry` was fixed in #4393
// (kernel publishes into `agents` before `name_index` on register, and
// unbinds `name_index` before retracting `agents` on remove), so an
// immediate `GET /api/agents` after a successful POST/DELETE *should* see
// the new state.
//
// In practice the read-after-write still goes through tokio's task
// scheduler, the axum service stack, and an extra tcp round-trip. A bare
// "fire requests then read once" assertion can race that pipeline on slow
// or loaded CI runners. To make these tests robust we poll the assertion
// target on a short interval until it converges or a generous timeout
// fires — *not* `tokio::time::pause()`, because the kernel runs real I/O
// (SQLite, tokio tasks) that a paused clock would deadlock.
const CONVERGENCE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// Polls `f` every [`POLL_INTERVAL`] until it returns `Some(value)` or
/// [`CONVERGENCE_TIMEOUT`] elapses, then returns the value (or panics with
/// `label` for diagnostics).
async fn poll_until<T, F, Fut>(label: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + CONVERGENCE_TIMEOUT;
    loop {
        if let Some(v) = f().await {
            return v;
        }
        if Instant::now() >= deadline {
            panic!(
                "poll_until({label}) did not converge within {:?}",
                CONVERGENCE_TIMEOUT
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors api_integration_test.rs)
// ---------------------------------------------------------------------------

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

/// Boot a real kernel, build the **full production router**, and serve it over
/// a real TCP socket on `127.0.0.1`.
///
/// Mirrors `start_full_router` in `api_integration_test.rs` for kernel/config
/// setup, but unlike that helper (which drives `app.oneshot(...)` in-process)
/// this one serves the router on an ephemeral port via
/// `into_make_service_with_connect_info::<SocketAddr>()` — the same call the
/// daemon uses in `server::serve`. The load tests need a real socket so their
/// `reqwest` + `tokio::spawn` concurrency exercises genuine HTTP throughput,
/// and the injected loopback `SocketAddr` is what lets the auth and rate-limit
/// middleware apply their documented same-host policy (open auth on loopback
/// with an empty `api_key`; rate-limit bypass for loopback callers that send no
/// forwarding header). See the module-level doc for the rate-limit rationale.
async fn start_test_server() -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    // Populate the model catalog in the temp home so the kernel boots with a
    // real registry (matches start_full_router in api_integration_test.rs).
    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
        None,
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        // Empty api_key => auth is open; combined with the genuine loopback
        // peer (below) the auth middleware passes requests without a token.
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().unwrap();

    let (app, state) = librefang_api::server::build_router(kernel, addr).await;

    tokio::spawn(async move {
        // SECURITY: `into_make_service_with_connect_info` injects the peer
        // SocketAddr so the auth + rate-limit middleware can recognize the
        // loopback caller — exactly as `server::serve` does for the daemon.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

const TEST_MANIFEST: &str = r#"
name = "load-test-agent"
version = "0.1.0"
description = "Load test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

// ---------------------------------------------------------------------------
// Load tests
// ---------------------------------------------------------------------------

/// Test: Concurrent agent spawns — verify kernel handles parallel agent creation.
#[tokio::test(flavor = "multi_thread")]
async fn load_concurrent_agent_spawns() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();
    let n = 20; // 20 concurrent spawns

    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..n {
        let c = client.clone();
        let url = format!("{}/api/agents", server.base_url);
        let manifest = TEST_MANIFEST.replace("load-test-agent", &format!("load-agent-{i}"));
        handles.push(tokio::spawn(async move {
            let res = c
                .post(&url)
                .json(&serde_json::json!({"manifest_toml": manifest}))
                .send()
                .await
                .expect("request failed");
            (res.status().as_u16(), i)
        }));
    }

    let mut success = 0;
    for h in handles {
        let (status, _i) = h.await.unwrap();
        if status == 200 || status == 201 {
            success += 1;
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  [LOAD] Concurrent spawns: {success}/{n} succeeded in {:.0}ms ({:.0} spawns/sec)",
        elapsed.as_millis(),
        n as f64 / elapsed.as_secs_f64()
    );
    assert!(success >= n - 2, "Most agents should spawn successfully");

    // Verify via list (paginated response: { items: [...], total, offset, limit }).
    //
    // Even though the kernel `register` path publishes into `agents` before
    // binding the name in `name_index` (see #4393), the read-after-write
    // here still crosses the HTTP boundary. Poll until the listing
    // converges to at least `success` entries rather than asserting on a
    // single snapshot — that snapshot races task scheduling on loaded CI
    // runners. See the helper comment block above.
    let count = poll_until("agents-list-after-spawn", || async {
        let resp: serde_json::Value = client
            .get(format!("{}/api/agents", server.base_url))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        let c = resp["items"].as_array().map(|a| a.len()).unwrap_or(0);
        if c >= success {
            Some(c)
        } else {
            None
        }
    })
    .await;
    eprintln!("  [LOAD] Total agents after spawn: {count}");
    assert!(count >= success);
}

/// Test: API endpoint latency — measure p50/p95/p99 for health, status, list agents.
#[tokio::test(flavor = "multi_thread")]
async fn load_endpoint_latency() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();

    // Spawn a few agents for the list endpoint to return
    for i in 0..5 {
        let manifest = TEST_MANIFEST.replace("load-test-agent", &format!("latency-agent-{i}"));
        client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
    }

    let endpoints = vec![
        ("GET", "/api/health"),
        ("GET", "/api/status"),
        ("GET", "/api/agents"),
        ("GET", "/api/tools"),
        ("GET", "/api/models"),
        ("GET", "/api/metrics"),
        ("GET", "/api/config"),
        ("GET", "/api/usage"),
    ];

    for (method, path) in &endpoints {
        let url = format!("{}{}", server.base_url, path);

        // Warmup: the first few requests for each endpoint pay a one-time
        // cost for lazy caches (agent registry snapshot, TLS session,
        // supervisor state) that doesn't reflect steady-state latency.
        // Without this, Windows CI sporadically blew the p99 budget because
        // a single 400-600ms cold-start sample dominated the 1% tail over
        // n=100. Warmup + a more stable percentile is how real load tests
        // handle shared-runner variance.
        for _ in 0..10 {
            let _ = match *method {
                "GET" => client.get(&url).send().await,
                _ => client.post(&url).send().await,
            };
        }

        let mut latencies = Vec::new();
        let n = 100;

        for _ in 0..n {
            let start = Instant::now();
            let res = match *method {
                "GET" => client.get(&url).send().await,
                _ => client.post(&url).send().await,
            };
            let elapsed = start.elapsed();
            assert!(res.is_ok(), "{method} {path} failed");
            latencies.push(elapsed);
        }

        latencies.sort();
        let p50 = latencies[n / 2];
        let p95 = latencies[(n as f64 * 0.95) as usize];
        let p99 = latencies[(n as f64 * 0.99) as usize];

        eprintln!(
            "  [LOAD] {method} {path:30} p50={:>5.1}ms  p95={:>5.1}ms  p99={:>5.1}ms",
            p50.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            p99.as_secs_f64() * 1000.0,
        );

        // Gate on p95 (5 samples out of 100) instead of p99 (1 sample): a
        // single-sample percentile on shared CI runners is dominated by GC
        // pauses and scheduler jitter, not real handler cost. Threshold is
        // deliberately loose (1s) — this is a smoke check that handlers
        // aren't pathologically slow, not a microbenchmark.
        assert!(
            p95 < Duration::from_millis(1000),
            "{method} {path} p95 too high: {p95:?}"
        );
    }
}

/// Test: Concurrent reads — many clients hitting the same endpoints simultaneously.
#[tokio::test(flavor = "multi_thread")]
async fn load_concurrent_reads() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();

    // Spawn some agents first
    for i in 0..3 {
        let manifest = TEST_MANIFEST.replace("load-test-agent", &format!("concurrent-agent-{i}"));
        client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
    }

    let n = 50;
    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..n {
        let c = client.clone();
        let base = server.base_url.clone();
        handles.push(tokio::spawn(async move {
            // Cycle through different endpoints
            let path = match i % 4 {
                0 => "/api/health",
                1 => "/api/agents",
                2 => "/api/status",
                _ => "/api/metrics",
            };
            let res = c
                .get(format!("{base}{path}"))
                .send()
                .await
                .expect("request failed");
            res.status().as_u16()
        }));
    }

    let mut success = 0;
    for h in handles {
        let status = h.await.unwrap();
        if status == 200 {
            success += 1;
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  [LOAD] Concurrent reads: {success}/{n} succeeded in {:.0}ms ({:.0} req/sec)",
        elapsed.as_millis(),
        n as f64 / elapsed.as_secs_f64()
    );
    assert_eq!(success, n, "All concurrent reads should succeed");
}

/// Test: Session management under load — create, list, and switch sessions.
#[tokio::test(flavor = "multi_thread")]
async fn load_session_management() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();

    // Spawn an agent
    let res: serde_json::Value = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = res["agent_id"].as_str().unwrap().to_string();

    // Create multiple sessions
    let n = 10;
    let start = Instant::now();
    let mut session_ids = Vec::new();

    for i in 0..n {
        let res: serde_json::Value = client
            .post(format!(
                "{}/api/agents/{}/sessions",
                server.base_url, agent_id
            ))
            .json(&serde_json::json!({"label": format!("session-{i}")}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(id) = res.get("session_id").and_then(|v| v.as_str()) {
            session_ids.push(id.to_string());
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  [LOAD] Created {n} sessions in {:.0}ms",
        elapsed.as_millis()
    );

    // List sessions
    let start = Instant::now();
    let sessions_resp: serde_json::Value = client
        .get(format!(
            "{}/api/agents/{}/sessions",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // Response is {"sessions": [...]} — extract the array
    let session_count = sessions_resp
        .get("sessions")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or_else(|| {
            // Fallback: maybe it's a direct array
            sessions_resp.as_array().map(|a| a.len()).unwrap_or(0)
        });
    eprintln!(
        "  [LOAD] Listed {session_count} sessions in {:.1}ms",
        start.elapsed().as_secs_f64() * 1000.0
    );

    // We expect at least some sessions (the original + our new ones)
    // Note: create_session might fail silently for some if agent was spawned without session
    eprintln!("  [LOAD] Session IDs collected: {}", session_ids.len());
    assert!(
        !session_ids.is_empty() || session_count > 0,
        "Should have created some sessions"
    );

    // Switch between sessions rapidly
    let start = Instant::now();
    for sid in &session_ids {
        client
            .post(format!(
                "{}/api/agents/{}/sessions/{}/switch",
                server.base_url, agent_id, sid
            ))
            .send()
            .await
            .unwrap();
    }
    eprintln!(
        "  [LOAD] Switched through {} sessions in {:.0}ms",
        session_ids.len(),
        start.elapsed().as_millis()
    );
}

/// Test: Workflow creation and listing under load.
#[tokio::test(flavor = "multi_thread")]
async fn load_workflow_operations() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();

    let n = 15;
    let start = Instant::now();

    // Create workflows concurrently
    let mut handles = Vec::new();
    for i in 0..n {
        let c = client.clone();
        let url = format!("{}/api/workflows", server.base_url);
        handles.push(tokio::spawn(async move {
            let res = c
                .post(&url)
                .json(&serde_json::json!({
                    "name": format!("wf-{i}"),
                    "description": format!("Load test workflow {i}"),
                    "steps": [{
                        "name": "step1",
                        "agent_name": "test-agent",
                        "mode": "sequential",
                        "prompt": "{{input}}"
                    }]
                }))
                .send()
                .await
                .expect("request failed");
            res.status().as_u16()
        }));
    }

    let mut created = 0;
    for h in handles {
        let status = h.await.unwrap();
        if status == 200 || status == 201 {
            created += 1;
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  [LOAD] Created {created}/{n} workflows in {:.0}ms",
        elapsed.as_millis()
    );

    // List all workflows
    let start = Instant::now();
    let workflows: serde_json::Value = client
        .get(format!("{}/api/workflows", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wf_count = workflows["items"].as_array().map(|a| a.len()).unwrap_or(0);
    eprintln!(
        "  [LOAD] Listed {wf_count} workflows in {:.1}ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    assert!(wf_count >= created);
}

/// Test: Agent spawn + kill cycle — stress the registry.
#[tokio::test(flavor = "multi_thread")]
async fn load_spawn_kill_cycle() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();

    let cycles = 10;
    let start = Instant::now();
    let mut ids = Vec::new();

    // Spawn
    for i in 0..cycles {
        let manifest = TEST_MANIFEST.replace("load-test-agent", &format!("cycle-agent-{i}"));
        let res: serde_json::Value = client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(id) = res.get("agent_id").and_then(|v| v.as_str()) {
            ids.push(id.to_string());
        }
    }

    // Kill (refs #4614: confirm required)
    for id in &ids {
        client
            .delete(format!(
                "{}/api/agents/{}?confirm=true",
                server.base_url, id
            ))
            .send()
            .await
            .unwrap();
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  [LOAD] Spawn+kill {cycles} agents in {:.0}ms ({:.0}ms per cycle)",
        elapsed.as_millis(),
        elapsed.as_millis() as f64 / cycles as f64
    );

    // Verify all cleaned up (paginated response: { items: [...], total, offset, limit }).
    //
    // The kernel `remove` path now unbinds `name_index` before retracting
    // from `agents` (see #4393), so the post-DELETE registry should be
    // monotonically consistent. Still, poll the HTTP listing until it
    // settles at exactly the default assistant — a single snapshot
    // assertion races scheduler/HTTP queueing on busy CI runners.
    let remaining = poll_until("agents-list-after-kill", || async {
        let resp: serde_json::Value = client
            .get(format!("{}/api/agents", server.base_url))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        let r = resp["items"].as_array().map(|a| a.len()).unwrap_or(0);
        if r == 1 {
            Some(r)
        } else {
            None
        }
    })
    .await;
    assert_eq!(remaining, 1, "Only default assistant should remain");
}

/// Test: Prometheus metrics endpoint under sustained load.
#[tokio::test(flavor = "multi_thread")]
async fn load_metrics_sustained() {
    let server = start_test_server().await;
    let client = librefang_kernel::http_client::new_client();

    // Spawn a few agents first so metrics have data
    for i in 0..3 {
        let manifest = TEST_MANIFEST.replace("load-test-agent", &format!("metrics-agent-{i}"));
        client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
    }

    // Hit metrics endpoint 200 times
    let n = 200;
    let start = Instant::now();
    for _ in 0..n {
        let res = client
            .get(format!("{}/api/metrics", server.base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let body = res.text().await.unwrap();
        assert!(body.contains("librefang_agents_active"));
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  [LOAD] Metrics {n} requests in {:.0}ms ({:.0} req/sec, {:.1}ms avg)",
        elapsed.as_millis(),
        n as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / n as f64
    );
}
