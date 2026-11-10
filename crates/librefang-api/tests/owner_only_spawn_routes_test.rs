//! Integration tests for the Owner gate on routes that reach a daemon-UID process spawn or a whole-config overwrite indirectly.
//!
//! `POST /api/mcp/servers` is Owner-only because a stdio transport is a raw `command` + `args` the daemon spawns.
//! Three Admin-reachable routes used to give the same capability one step removed:
//!   - `POST` / `PUT /api/registry/content/mcp` writes the on-disk MCP catalog entry, transport included.
//!   - `POST /api/extensions/install` (an alias of `POST /api/mcp/servers`) installs that catalog entry and reloads MCP, which spawns it.
//!   - `POST` / `PUT /api/registry/content/plugin` writes a plugin manifest that is enabled by default.
//!
//! `POST /api/restore` writes the archive's config.toml (master key, `[[users]]`, MCP commands) and so is a superset of the Owner-only `/api/config` + `/api/users*` writes.
//!
//! The harness mirrors `vault_routes_integration.rs`: the real route modules behind the production auth middleware, driven with `tower::oneshot`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use librefang_api::middleware;
use librefang_api::routes;
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

const MASTER_KEY: &str = "spawn-gate-master-key";
const OWNER_KEY: &str = "carol-spawn-gate-owner-key";
const ADMIN_KEY: &str = "alice-spawn-gate-admin-key";

struct Harness {
    app: Router,
    state: Arc<routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

fn build_harness() -> Harness {
    let users = [("Carol", "owner", OWNER_KEY), ("Alice", "admin", ADMIN_KEY)];
    let mut user_configs: Vec<UserConfig> = Vec::with_capacity(users.len());
    let mut api_user_records: Vec<middleware::ApiUserAuth> = Vec::with_capacity(users.len());
    for (name, role_str, key) in users {
        let hash =
            librefang_api::password_hash::hash_password(key).expect("password hash should succeed");
        user_configs.push(UserConfig {
            name: name.to_string(),
            role: role_str.to_string(),
            channel_bindings: std::collections::HashMap::new(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        api_user_records.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role_str),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = MASTER_KEY.to_string();
        cfg.users = user_configs;
    }))
    .with_api_key(MASTER_KEY)
    .with_user_api_keys(api_user_records);

    let (state, tmp, _cfg_path) = test.into_parts();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: state.dashboard_auth_enabled.clone(),
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        allow_no_auth: true,
        audit_log: Some(state.kernel.audit().clone()),
        a2a_agents_path: None,
    };

    let api = Router::new()
        .merge(routes::registry::router())
        .merge(routes::skills::router())
        .merge(routes::backup::router());
    let app = Router::new()
        .nest("/api", api)
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn send(
    app: Router,
    method: Method,
    path: &str,
    bearer: &str,
    body: serde_json::Value,
) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

fn stdio_catalog_entry(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "description": "spawn gate fixture",
        "transport": {"type": "stdio", "command": "sh", "args": ["-c", "exit 0"]}
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_cannot_reach_spawn_or_config_overwrite_aliases() {
    let h = build_harness();
    let home = h.state.kernel.home_dir().to_path_buf();

    let cases: Vec<(Method, &str, serde_json::Value)> = vec![
        (
            Method::POST,
            "/api/registry/content/mcp",
            stdio_catalog_entry("admin-mcp"),
        ),
        (
            Method::PUT,
            "/api/registry/content/mcp",
            stdio_catalog_entry("admin-mcp"),
        ),
        (
            Method::POST,
            "/api/registry/content/plugin",
            serde_json::json!({"name": "admin-plugin", "version": "0.1.0"}),
        ),
        (
            Method::PUT,
            "/api/registry/content/plugin",
            serde_json::json!({"name": "admin-plugin", "version": "0.1.0"}),
        ),
        // Percent-encoded spellings of the content type: axum decodes the capture to "mcp" / "plugin", so the gate must too.
        (
            Method::POST,
            "/api/registry/content/%6Dcp",
            stdio_catalog_entry("admin-mcp"),
        ),
        (
            Method::PUT,
            "/api/registry/content/plug%69n",
            serde_json::json!({"name": "admin-plugin", "version": "0.1.0"}),
        ),
        (
            Method::POST,
            "/api/extensions/install",
            serde_json::json!({"name": "admin-mcp"}),
        ),
        (
            Method::POST,
            "/api/restore",
            serde_json::json!({"filename": "librefang_backup_old.zip"}),
        ),
    ];
    for (method, path, body) in cases {
        let (status, resp) = send(h.app.clone(), method.clone(), path, ADMIN_KEY, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Admin must not {method} {path}: {}",
            String::from_utf8_lossy(&resp)
        );
    }
    assert!(!home.join("mcp/catalog/admin-mcp.toml").exists());
    assert!(!home.join("plugins/admin-plugin/plugin.toml").exists());

    // The gate is Owner, not "nobody": the Owner's registry write lands.
    let (status, resp) = send(
        h.app.clone(),
        Method::POST,
        "/api/registry/content/mcp",
        OWNER_KEY,
        stdio_catalog_entry("owner-mcp"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Owner write must succeed: {}",
        String::from_utf8_lossy(&resp)
    );
    assert!(home.join("mcp/catalog/owner-mcp.toml").exists());
}
