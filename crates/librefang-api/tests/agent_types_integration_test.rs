//! Integration tests for agent-types CRUD and ephemeral-agent spawn routes.
//!
//! Covered routes (refs #6699):
//!   * `GET    /api/templates`          — list all agent types
//!   * `GET    /api/templates/{name}`   — get one agent type
//!   * `POST   /api/templates`          — create an agent type
//!   * `PUT    /api/templates/{name}`   — update an agent type
//!   * `DELETE /api/templates/{name}`   — delete an agent type
//!
//! These tests boot a real `LibreFangKernel` via `MockKernelBuilder` (no
//! networking, no LLM credentials) and drive the agent-types router via
//! `tower::ServiceExt::oneshot`.

// The process-wide `crud_lock` is intentionally held across the whole test
// body (including awaits): it serializes CRUD tests against each other so
// they do not interleave kernel state. A test-only std mutex, held for the
// test's duration, is exactly its purpose.
#![allow(clippy::await_holding_lock)]

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

/// One tempdir for the whole test binary, set once via OnceLock — the
/// same pattern as `profiles_templates_routes_integration.rs`. Setting
/// `LIBREFANG_HOME` per-test races with every other concurrent test
/// that calls `librefang_home()` (#6931 review).
fn agent_types_home() -> &'static tempfile::TempDir {
    static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
    HOME.get_or_init(|| {
        let tmp = tempfile::tempdir().expect("tempdir for agent-types test");
        // Safety: set once, before any concurrent reader — the standard
        // workspace pattern for env-var-driven tests (Rust 2024 edition
        // requires the unsafe block for env mutation).
        std::env::set_var("LIBREFANG_HOME", tmp.path());
        tmp
    })
}

/// Serialise CRUD tests so concurrent creates don't observe each other's
/// fixtures when listing (list walks the whole templates dir).
fn crud_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

async fn boot() -> Harness {
    let _home = agent_types_home();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let state = test.app_state();
    let app = routes::agent_templates::router().with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

fn agent_type_json(name: &str, desc: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": desc,
        "system_prompt": "You are a test agent.",
        "provider": "test-provider",
        "model": "test-model",
        "tools": ["file_read", "web_fetch"],
        "skills": ["test-skill"]
    })
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_list_returns_200() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Create → read → update → delete lifecycle
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_crud_lifecycle() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = agent_type_json("test-agent-crud", "A test agent for CRUD.");
    let body_bytes = serde_json::to_vec(&body).unwrap();

    // Create
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(body_bytes.clone()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Read the one we just created
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched["name"], "test-agent-crud");
    assert_eq!(fetched["provider"], "test-provider");
    assert_eq!(fetched["model"], "test-model");
    assert!(fetched["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "file_read"));
    assert!(fetched["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "test-skill"));

    // Update — change description and tools
    let updated = serde_json::json!({
        "name": "test-agent-crud",
        "description": "Updated description.",
        "system_prompt": "You are an updated test agent.",
        "provider": "updated-provider",
        "model": "updated-model",
        "tools": ["shell_exec"],
        "skills": ["updated-skill"]
    });
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/test-agent-crud")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&updated).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched["name"], "test-agent-crud");
    assert_eq!(fetched["provider"], "updated-provider");
    assert_eq!(fetched["model"], "updated-model");
    assert!(fetched["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "shell_exec"));
    assert!(fetched["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "updated-skill"));

    // Delete
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Confirm deleted — should 404
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_missing_name() {
    let h = boot().await;
    let body = serde_json::json!({"description": "no name"});
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_rejects_path_traversal() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/../../../etc/passwd")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_invalid_name_chars() {
    let h = boot().await;
    let body = agent_type_json("bad name", "has spaces");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_duplicate() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = agent_type_json("duplicate-test", "first create");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Same name again → 409
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/duplicate-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_get_nonexistent_returns_404() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/nonexistent-type")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_list_includes_created_items() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = agent_type_json("list-include-test", "for list test");
    // Create it
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let _ = h.app.clone().oneshot(req).await.unwrap();

    // List — should include it
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(list["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "list-include-test"));

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/list-include-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

// ---------------------------------------------------------------------------
// TOML injection — ensure special characters are escaped
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_escapes_toml_special_chars() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = serde_json::json!({
        "name": "toml-inject-test",
        "description": "desc with \"quotes\" and \\ backslashes and \n newlines",
        "system_prompt": "prompt with \"quotes\"",
        "provider": "test",
        "model": "test",
        "tools": ["file_read"],
        "skills": []
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    // Must not 500 — the TOML serializer should escape everything
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Read back — round-trip should preserve the content
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/toml-inject-test")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        fetched["description"],
        "desc with \"quotes\" and \\ backslashes and \n newlines"
    );

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/toml-inject-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

// ---------------------------------------------------------------------------
// Production-router coverage (#6931 review)
//
// Everything above drives `routes::agent_templates::router()` directly. That
// proves the handlers behave, and proves nothing about whether `server.rs`
// ever mounts them: delete the `.merge()` in `api_v1_routes()` and every test
// above still passes while the endpoints 404 in the running daemon.
//
// The two tests below close that gap by driving `server::build_router` — the
// same call the daemon makes — so a missing registration fails here rather
// than on someone's install.
// ---------------------------------------------------------------------------

struct ProdHarness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for ProdHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Build the production router with RBAC users wired in.
///
/// Each tuple is `(name, role, api_key)`. `allow_no_auth` stays false so an
/// unauthenticated request is rejected at the middleware layer, which is what
/// makes the role assertions below mean anything.
async fn boot_production_router(api_key: &str, users: Vec<(&str, &str, &str)>) -> ProdHarness {
    use librefang_api::{middleware, server};
    use librefang_kernel::auth::UserRole as KernelUserRole;
    use librefang_types::agent::UserId;
    use librefang_types::config::{DefaultModelConfig, KernelConfig, UserConfig};

    let tmp = tempfile::tempdir().expect("tempdir for production-router harness");

    let mut user_configs: Vec<UserConfig> = Vec::with_capacity(users.len());
    let mut api_user_records: Vec<middleware::ApiUserAuth> = Vec::with_capacity(users.len());
    for (name, role_str, key) in &users {
        let hash =
            librefang_api::password_hash::hash_password(key).expect("password hash should succeed");
        user_configs.push(UserConfig {
            name: (*name).to_string(),
            role: (*role_str).to_string(),
            channel_bindings: std::collections::HashMap::new(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        api_user_records.push(middleware::ApiUserAuth {
            name: (*name).to_string(),
            role: KernelUserRole::from_str_role(role_str),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        users: user_configs,
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

    let kernel =
        librefang_kernel::LibreFangKernel::boot_with_config(config).expect("kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;
    *state.user_api_keys.write().await = api_user_records;

    ProdHarness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn prod_request(
    h: &ProdHarness,
    method: Method,
    path: &str,
    api_key: Option<&str>,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(key) = api_key {
        builder = builder.header("Authorization", format!("Bearer {key}"));
    }
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).expect("body serialises")
        }
        None => Vec::new(),
    };
    let req = builder
        .body(Body::from(body_bytes))
        .expect("request builds");
    h.app
        .clone()
        .oneshot(req)
        .await
        .expect("router responds")
        .status()
}

/// Both agent-type paths are reachable through the router the daemon builds.
///
/// The probe is a method the path deliberately does *not* serve. axum answers
/// 405 when it knows the path but not the method, and 404 when it does not
/// know the path at all — so the two cases separate cleanly.
///
/// Asking a served method instead cannot tell them apart: `GET
/// /api/templates/does-not-exist` answers 404 whether the route is missing or
/// merely the agent type is, which is what an earlier version of this test got
/// wrong. It asserted "anything but 404 means mounted" and would have passed a
/// deleted merge.
#[tokio::test(flavor = "multi_thread")]
async fn agent_type_routes_are_registered_in_the_production_router() {
    let h = boot_production_router("admin-key", vec![("admin", "admin", "admin-key")]).await;

    for (unserved_method, path, served) in [
        // `/api/templates` serves GET and POST.
        (Method::DELETE, "/api/templates", "GET, POST"),
        // `/api/templates/{name}` serves GET, PUT and DELETE.
        (
            Method::POST,
            "/api/templates/does-not-exist",
            "GET, PUT, DELETE",
        ),
    ] {
        let status = prod_request(&h, unserved_method.clone(), path, Some("admin-key"), None).await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{unserved_method} {path} should be 405 because the path is mounted \
             and serves {served}; a 404 means server::build_router never merged \
             it and the daemon would 404 while every direct-router test above \
             still passes"
        );
    }
}

/// The served methods reach their handlers rather than a router miss.
///
/// Registration alone is not enough: a path can be mounted with the wrong
/// method set and still 405 the one the dashboard actually calls. This pins
/// the list and create verbs, which is what the editor needs on load and save.
#[tokio::test(flavor = "multi_thread")]
async fn agent_type_list_and_create_verbs_reach_their_handlers() {
    let h = boot_production_router("admin-key", vec![("admin", "admin", "admin-key")]).await;

    let listed = prod_request(&h, Method::GET, "/api/templates", Some("admin-key"), None).await;
    assert_eq!(listed, StatusCode::OK, "GET /api/templates must list");

    // A create with a body the handler rejects still proves POST is routed:
    // the handler answered, so the verb is not 405 and not a router miss.
    let created = prod_request(
        &h,
        Method::POST,
        "/api/templates",
        Some("admin-key"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_ne!(
        created,
        StatusCode::METHOD_NOT_ALLOWED,
        "POST /api/templates reached the router but not the create handler"
    );
    assert_ne!(
        created,
        StatusCode::NOT_FOUND,
        "POST /api/templates is not mounted at all"
    );
}

/// An unauthenticated caller cannot write agent types.
///
/// The write endpoints create, overwrite and delete manifests that spawn
/// agents, so the interesting assertion is that they are not reachable
/// without credentials — the read path is deliberately not asserted here
/// because `require_auth_for_reads` is a separate policy.
#[tokio::test(flavor = "multi_thread")]
async fn agent_type_writes_reject_an_unauthenticated_caller() {
    let h = boot_production_router("admin-key", vec![("admin", "admin", "admin-key")]).await;

    for (method, path, body) in [
        (
            Method::POST,
            "/api/templates",
            Some(serde_json::json!({"name": "unauthorised"})),
        ),
        (
            Method::PUT,
            "/api/templates/unauthorised",
            Some(serde_json::json!({"name": "unauthorised"})),
        ),
        (Method::DELETE, "/api/templates/unauthorised", None),
    ] {
        let status = prod_request(&h, method.clone(), path, None, body).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
            "{method} {path} without credentials returned {status}, expected 401 or 403"
        );
    }
}
