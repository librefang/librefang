//! Memory admin-only endpoint isolation tests.
//!
//! Verifies that admin-only memory endpoints return 403 FORBIDDEN for
//! non-admin tenants and remain reachable to configured admin accounts carrying
//! a concrete X-Account-Id.
//!
//! Run: cargo test -p librefang-api --test memory_admin_tests -- --nocapture

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Test infrastructure (mirrored from account_tests.rs)
// ---------------------------------------------------------------------------

struct MtHarness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for MtHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl MtHarness {
    /// Send a request through the router. Clones the router internally so the
    /// harness can be reused for multiple requests in the same test.
    async fn send(&self, req: Request<Body>) -> axum::response::Response {
        self.app.clone().oneshot(req).await.unwrap()
    }
}

/// Boot a full router with multi-tenant mode enabled.
async fn start_mt_router() -> MtHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    MtHarness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Boot a full router with multi-tenant mode enabled and an admin account configured.
async fn start_mt_router_with_admin(admin_id: &str) -> MtHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        admin_accounts: vec![admin_id.to_string()],
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    MtHarness {
        app,
        state,
        _tmp: tmp,
    }
}

// ---------------------------------------------------------------------------
// Test 1: POST /api/memory/cleanup with tenant account -> 403 FORBIDDEN
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn tenant_cleanup_returns_403() {
    let h = start_mt_router().await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/memory/cleanup")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/memory/cleanup must return 403 for tenants, got: {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Test 2: POST /api/memory/decay with tenant account -> 403 FORBIDDEN
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn tenant_decay_returns_403() {
    let h = start_mt_router().await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/memory/decay")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/memory/decay must return 403 for tenants, got: {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Test 3: POST /api/memory/cleanup with configured admin account -> NOT 403
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn admin_cleanup_not_forbidden() {
    let h = start_mt_router_with_admin("tenant-admin").await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/memory/cleanup")
                .header("x-account-id", "tenant-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    // Admin should never get 403. The handler may return 200 (cleanup ran)
    // or 500 (proactive memory store not initialized) — both are acceptable.
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/memory/cleanup must NOT return 403 for configured admin, got: {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Test 4: POST /api/memory/decay with configured admin account -> NOT 403
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn admin_decay_not_forbidden() {
    let h = start_mt_router_with_admin("tenant-admin").await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/memory/decay")
                .header("x-account-id", "tenant-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    // Admin should never get 403. May get 200 or 500 depending on PM store
    // availability — the point is the admin guard does not fire.
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/memory/decay must NOT return 403 for configured admin, got: {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Test 5: GET /api/memory/config with configured admin account -> 200
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn admin_config_get_returns_200() {
    let h = start_mt_router_with_admin("tenant-admin").await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory/config")
                .header("x-account-id", "tenant-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /api/memory/config should return 200 for configured admin, got: {}",
        resp.status()
    );

    // Verify response structure contains expected config keys
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json.get("proactive_memory").is_some(),
        "Config response should contain proactive_memory section, got: {json}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: PATCH /api/memory/config with configured admin account -> NOT 403
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn admin_config_patch_not_forbidden() {
    let h = start_mt_router_with_admin("tenant-admin").await;

    let resp = h
        .send(
            Request::builder()
                .method("PATCH")
                .uri("/api/memory/config")
                .header("x-account-id", "tenant-admin")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "decay_rate": 0.05
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    // Admin should not get 403. Acceptable responses are 200 (config updated)
    // or 400/500 (config file parse error in temp dir) — never 403.
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "PATCH /api/memory/config must NOT return 403 for configured admin, got: {}",
        resp.status()
    );
}
