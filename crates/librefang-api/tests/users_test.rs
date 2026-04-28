//! Integration tests for the RBAC user-management endpoints.
//!
//! These exercise the real `users` router against a freshly-booted kernel
//! backed by a temp-dir `config.toml`, then walk through CRUD and the CSV-
//! style bulk import preview/commit dance. We avoid the full router so the
//! tests stay fast and don't need any LLM credentials.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig, UserConfig};
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

async fn boot() -> Harness {
    boot_with_seed_users(vec![]).await
}

async fn boot_with_seed_users(seed: Vec<UserConfig>) -> Harness {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        users: seed,
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    // Persist the seed config so persist_users round-trips through a real
    // file on disk (mirrors how the daemon runs in production).
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

    let kernel = LibreFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        skillhub_cache: dashmap::DashMap::new(),
        provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
        webhook_store: librefang_api::webhook_store::WebhookStore::load(std::env::temp_dir().join(
            format!("librefang-test-users-{}.json", uuid::Uuid::new_v4()),
        )),
        active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        #[cfg(feature = "telemetry")]
        prometheus_handle: None,
        media_drivers: librefang_runtime::media::MediaDriverCache::new(),
        webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
        api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
        user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        provider_test_cache: dashmap::DashMap::new(),
        config_write_lock: tokio::sync::Mutex::new(()),
        pending_a2a_agents: dashmap::DashMap::new(),
        auth_login_limiter: std::sync::Arc::new(librefang_api::rate_limiter::AuthLoginLimiter::new(0)),
    });

    let app = Router::new()
        .nest("/api", routes::users::router())
        .with_state(state.clone());

    Harness {
        app,
        _state: state,
        _tmp: tmp,
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

#[tokio::test(flavor = "multi_thread")]
async fn users_list_starts_empty() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/users", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn users_create_then_get_then_delete_round_trips() {
    let h = boot().await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({
            "name": "Alice",
            "role": "admin",
            "channel_bindings": {"telegram": "111"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create: {body:?}");
    assert_eq!(body["name"], "Alice");
    assert_eq!(body["role"], "admin");
    assert_eq!(body["channel_bindings"]["telegram"], "111");
    assert_eq!(body["has_api_key"], false);

    // GET single
    let (status, body) = json_request(&h, Method::GET, "/api/users/Alice", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Alice");

    // Reload picked it up — list should now contain Alice
    let (status, body) = json_request(&h, Method::GET, "/api/users", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    // DELETE
    let (status, _) = json_request(&h, Method::DELETE, "/api/users/Alice", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = json_request(&h, Method::GET, "/api/users/Alice", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn users_create_rejects_invalid_role() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({"name": "Bob", "role": "wizard"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("invalid role"),
        "got: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn users_create_rejects_duplicate() {
    let h = boot().await;
    let payload = serde_json::json!({"name": "Carol", "role": "user"});
    let (status, _) = json_request(&h, Method::POST, "/api/users", Some(payload.clone())).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = json_request(&h, Method::POST, "/api/users", Some(payload)).await;
    assert_eq!(status, StatusCode::CONFLICT, "got: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn users_update_changes_role_and_bindings() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({"name": "Dan", "role": "user"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Dan",
        Some(serde_json::json!({
            "name": "Dan",
            "role": "viewer",
            "channel_bindings": {"discord": "222"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update: {body:?}");
    assert_eq!(body["role"], "viewer");
    assert_eq!(body["channel_bindings"]["discord"], "222");
}

#[tokio::test(flavor = "multi_thread")]
async fn users_update_unknown_returns_404() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/users/Ghost",
        Some(serde_json::json!({"name": "Ghost", "role": "user"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn users_import_dry_run_reports_counts() {
    let h = boot().await;
    // Seed one user so we can confirm "updated" counting.
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({"name": "Eve", "role": "user"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users/import",
        Some(serde_json::json!({
            "dry_run": true,
            "rows": [
                {"name": "Eve", "role": "admin"},
                {"name": "Frank", "role": "user", "channel_bindings": {"slack": "U999"}},
                {"name": "BadRole", "role": "wizard"},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["dry_run"], true);
    assert_eq!(body["created"], 1);
    assert_eq!(body["updated"], 1);
    assert_eq!(body["failed"], 1);

    // Dry run must not have written anything.
    let (_, list) = json_request(&h, Method::GET, "/api/users", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn users_import_commit_persists_rows() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users/import",
        Some(serde_json::json!({
            "dry_run": false,
            "rows": [
                {"name": "Gina", "role": "admin", "channel_bindings": {"telegram": "11"}},
                {"name": "Hank", "role": "user"},
                {"name": "Bad", "role": "nope"},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["created"], 2);
    assert_eq!(body["failed"], 1);

    let (_, list) = json_request(&h, Method::GET, "/api/users", None).await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Gina"));
    assert!(names.contains(&"Hank"));
}

/// PR #3209 review item — the wire `api_key_hash` must be a valid
/// Argon2id PHC string. Without this check an Owner could paste an
/// arbitrary value (constant, exfiltrated hash, empty-after-trim) into
/// `api_key_hash` and silently grant whoever knows that hash's preimage
/// a working API key.
#[tokio::test(flavor = "multi_thread")]
async fn users_create_rejects_invalid_api_key_hash() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({
            "name": "Mallory",
            "role": "user",
            "api_key_hash": "not-a-real-hash"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("Argon2"),
        "error must mention Argon2 PHC requirement: {body:?}"
    );

    // A genuine Argon2id hash IS accepted.
    let real_hash = librefang_api::password_hash::hash_password("supersecret").expect("hash");
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({
            "name": "Mallory",
            "role": "user",
            "api_key_hash": real_hash,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

/// PR #3209 re-review — the M6 dashboard's `PUT /api/users/{name}` MUST
/// preserve the RBAC M3 (#3205) per-user policy fields (`tool_policy`,
/// `tool_categories`, `memory_access`, `channel_tool_rules`) across an
/// edit it doesn't itself surface. Without the preserve-and-merge in
/// `update_user`, an admin retitling a Viewer would silently flip
/// `pii_access` back to `false`-via-default and disable the
/// per-user tool policy. Same coverage for the bulk-import update path.
#[tokio::test(flavor = "multi_thread")]
async fn users_update_and_import_preserve_rbac_m3_policy_fields() {
    use librefang_types::user_policy::{UserMemoryAccess, UserToolPolicy};
    use std::collections::HashMap;

    // Seed a user with non-default RBAC M3 fields the M6 dashboard
    // doesn't expose. The kernel boots from this config and the
    // on-disk `config.toml` round-trips it.
    let seed = UserConfig {
        name: "Bob".into(),
        role: "viewer".into(),
        channel_bindings: {
            let mut m = HashMap::new();
            m.insert("telegram".into(), "111".into());
            m
        },
        api_key_hash: None,
        budget: None,
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec!["shell_exec".into()],
        }),
        tool_categories: None,
        memory_access: Some(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        }),
        channel_tool_rules: HashMap::new(),
    };
    let h = boot_with_seed_users(vec![seed.clone()]).await;

    // 1. Direct PUT — admin retitles Bob (rename + role change). The
    //    request body never mentions tool_policy / memory_access; the
    //    server must fill them in from the pre-existing config.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/users/Bob",
        Some(serde_json::json!({
            "name": "BobRenamed",
            "role": "user",
            "channel_bindings": {"telegram": "111"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after_put = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "BobRenamed")
        .cloned()
        .expect("renamed user must exist");
    assert_eq!(after_put.role, "user", "role change applied");
    assert_eq!(
        after_put.tool_policy, seed.tool_policy,
        "tool_policy was clobbered by PUT"
    );
    assert_eq!(
        after_put.memory_access, seed.memory_access,
        "memory_access (incl. pii_access=false) was clobbered by PUT"
    );

    // 2. Bulk-import update — same user, no policy fields in the CSV
    //    payload. The import path's "if name matches existing" branch
    //    must also preserve the RBAC M3 fields.
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/users/import",
        Some(serde_json::json!({
            "dry_run": false,
            "rows": [
                {"name": "BobRenamed", "role": "admin"},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after_import = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "BobRenamed")
        .cloned()
        .expect("user must still exist after import");
    assert_eq!(after_import.role, "admin", "import applied role bump");
    assert_eq!(
        after_import.tool_policy, seed.tool_policy,
        "tool_policy was clobbered by bulk import"
    );
    assert_eq!(
        after_import.memory_access, seed.memory_access,
        "memory_access was clobbered by bulk import"
    );
}

/// PR #3203 (RBAC M5) regression — `UserConfig.budget` must survive
/// PUT and CSV-reimport edits the same way the M3 policy fields do.
/// Without preserve logic, `..new_u.clone()` in the import-update branch
/// silently zeroes a per-user spend cap because CSV rows always carry
/// `budget: None`.
#[tokio::test(flavor = "multi_thread")]
async fn users_update_and_import_preserve_m5_budget() {
    use librefang_types::config::UserBudgetConfig;
    use std::collections::HashMap;

    let seeded_budget = UserBudgetConfig {
        max_hourly_usd: 1.0,
        max_daily_usd: 10.0,
        max_monthly_usd: 100.0,
        alert_threshold: 0.75,
    };
    let seed = UserConfig {
        name: "Carol".into(),
        role: "user".into(),
        channel_bindings: HashMap::new(),
        api_key_hash: None,
        budget: Some(seeded_budget.clone()),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed.clone()]).await;

    // 1. PUT — name + role edit, no budget in payload. Server must
    //    fill budget back from the pre-existing config.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/users/Carol",
        Some(serde_json::json!({
            "name": "CarolRenamed",
            "role": "admin",
            "channel_bindings": {}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after_put = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "CarolRenamed")
        .cloned()
        .expect("renamed user must exist");
    assert_eq!(after_put.role, "admin", "role change applied");
    assert_eq!(
        after_put.budget,
        Some(seeded_budget.clone()),
        "budget was clobbered by PUT — per-user spend cap silently wiped"
    );

    // 2. Bulk-import update — same user, no budget in CSV row. The
    //    import path's "if name matches existing" branch must also
    //    preserve the budget.
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/users/import",
        Some(serde_json::json!({
            "dry_run": false,
            "rows": [
                {"name": "CarolRenamed", "role": "user"},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after_import = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "CarolRenamed")
        .cloned()
        .expect("user must still exist after import");
    assert_eq!(after_import.role, "user", "import applied role change");
    assert_eq!(
        after_import.budget,
        Some(seeded_budget),
        "budget was clobbered by bulk import — ..new_u.clone() bug regressed"
    );
}

/// PR #3209 review item — `persist_users` MUST refuse to overwrite a
/// corrupt `config.toml` rather than silently replacing it with a doc
/// containing only `[[users]]` (which would erase the operator's
/// agents / providers / taint rules etc.).
#[tokio::test(flavor = "multi_thread")]
async fn users_create_refuses_to_overwrite_corrupt_config_toml() {
    let h = boot().await;

    // Corrupt the on-disk config file — kernel still has the previous
    // good copy in memory, but the next `persist_users` call has to
    // round-trip through the file.
    let config_path = h._tmp.path().join("config.toml");
    std::fs::write(&config_path, "this is not [[ valid TOML\nbroken = =\n")
        .expect("seed corrupt config");

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({
            "name": "Postcorrupt",
            "role": "user",
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "expected 500 on corrupt config, got: {body:?}"
    );
    assert!(
        body["error"].as_str().unwrap_or("").contains("config.toml"),
        "error should mention config.toml: {body:?}"
    );

    // The corrupt file must still be on disk verbatim — we have NOT
    // silently replaced it with a stub document.
    let on_disk = std::fs::read_to_string(&config_path).expect("read");
    assert!(
        on_disk.contains("this is not [[ valid TOML"),
        "config.toml was overwritten despite parse failure: {on_disk}"
    );
}

// ---------------------------------------------------------------------------
// RBAC M3 (#3205) per-user policy GET / PUT — exercises the M6 follow-up
// that wires the dashboard's matrix editor to the real daemon endpoint.
// ---------------------------------------------------------------------------

/// Seed a user with non-default `tool_policy` + `memory_access`. GET must
/// surface every field verbatim so the dashboard can render the editor
/// without a second round-trip.
#[tokio::test(flavor = "multi_thread")]
async fn users_policy_get_round_trip() {
    use librefang_types::user_policy::{UserMemoryAccess, UserToolPolicy};
    use std::collections::HashMap;

    let seed = UserConfig {
        name: "Bob".into(),
        role: "user".into(),
        channel_bindings: HashMap::new(),
        api_key_hash: None,
        budget: None,
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_*".into()],
            denied_tools: vec!["shell_exec".into()],
        }),
        tool_categories: None,
        memory_access: Some(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into(), "kv:bob".into()],
            writable_namespaces: vec!["kv:bob".into()],
            pii_access: true,
            export_allowed: false,
            delete_allowed: true,
        }),
        channel_tool_rules: HashMap::new(),
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, body) = json_request(&h, Method::GET, "/api/users/Bob/policy", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["tool_policy"]["allowed_tools"],
        serde_json::json!(["web_*"])
    );
    assert_eq!(
        body["tool_policy"]["denied_tools"],
        serde_json::json!(["shell_exec"])
    );
    assert_eq!(
        body["memory_access"]["readable_namespaces"],
        serde_json::json!(["proactive", "kv:bob"])
    );
    assert_eq!(
        body["memory_access"]["writable_namespaces"],
        serde_json::json!(["kv:bob"])
    );
    assert_eq!(body["memory_access"]["pii_access"], true);
    assert_eq!(body["memory_access"]["delete_allowed"], true);
    assert_eq!(body["memory_access"]["export_allowed"], false);
    assert!(body["tool_categories"].is_null(), "{body:?}");
    assert_eq!(body["channel_tool_rules"], serde_json::json!({}));
}

/// PUT with `tool_policy: null` must clear that slot but leave
/// `memory_access` (which the request body never mentioned) untouched.
/// Pins the absent-vs-null distinction the handler relies on.
#[tokio::test(flavor = "multi_thread")]
async fn users_policy_put_replaces_only_specified_fields() {
    use librefang_types::user_policy::{UserMemoryAccess, UserToolPolicy};
    use std::collections::HashMap;

    let seed_memory = UserMemoryAccess {
        readable_namespaces: vec!["proactive".into()],
        writable_namespaces: vec![],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    };
    let seed = UserConfig {
        name: "Carol".into(),
        role: "user".into(),
        channel_bindings: HashMap::new(),
        api_key_hash: None,
        budget: None,
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_*".into()],
            denied_tools: vec![],
        }),
        tool_categories: None,
        memory_access: Some(seed_memory.clone()),
        channel_tool_rules: HashMap::new(),
    };
    let h = boot_with_seed_users(vec![seed]).await;

    // PUT clears tool_policy, leaves memory_access alone (key absent).
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Carol/policy",
        Some(serde_json::json!({ "tool_policy": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["tool_policy"].is_null(), "tool_policy must be cleared");

    // Verify in-kernel state — memory_access must still match the seed.
    let after = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "Carol")
        .cloned()
        .expect("Carol must still exist");
    assert!(
        after.tool_policy.is_none(),
        "tool_policy was not cleared: {:?}",
        after.tool_policy
    );
    assert_eq!(
        after.memory_access,
        Some(seed_memory),
        "memory_access was clobbered despite being absent from PUT body"
    );
}

/// `writable_namespaces` MUST be a subset of `readable_namespaces`. There
/// is no upstream enforcement for this invariant today; the handler is the
/// first gate. Pins the new validation so a refactor doesn't silently drop it.
#[tokio::test(flavor = "multi_thread")]
async fn users_policy_put_validates_writable_subset_of_readable() {
    use std::collections::HashMap;
    let seed = UserConfig {
        name: "Dan".into(),
        role: "user".into(),
        channel_bindings: HashMap::new(),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Dan/policy",
        Some(serde_json::json!({
            "memory_access": {
                "readable_namespaces": [],
                "writable_namespaces": ["proactive"],
                "pii_access": false,
                "export_allowed": false,
                "delete_allowed": false
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.contains("subset") || err.contains("readable"),
        "error must mention the readable/writable subset rule: {err}"
    );
}

/// Empty / whitespace-only tool entries are rejected — without this an
/// operator can paste a stray newline into the matrix editor and grant
/// `""` (matches every tool via the glob layer).
#[tokio::test(flavor = "multi_thread")]
async fn users_policy_put_validates_no_empty_tool_strings() {
    use std::collections::HashMap;
    let seed = UserConfig {
        name: "Eve".into(),
        role: "user".into(),
        channel_bindings: HashMap::new(),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Eve/policy",
        Some(serde_json::json!({
            "tool_policy": {
                "allowed_tools": ["", "web_search"],
                "denied_tools": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("empty"),
        "error must mention the empty entry: {body:?}"
    );
}

/// `channel_tool_rules` keys land verbatim in `config.toml` and are
/// matched against channel-adapter identifiers (`telegram`, `slack`,
/// `feishu`, …). The original `trim().is_empty()` check let through:
///   - embedded newlines / control chars (`"foo\nbar"` survives `trim`)
///   - 10 KB blobs that bloat the TOML round-trip
///   - non-ASCII or whitespace-bearing names that never match an adapter
/// Each invalid shape must be rejected at the PUT boundary; valid slugs
/// like `feishu` must still round-trip.
#[tokio::test(flavor = "multi_thread")]
async fn users_policy_put_validates_channel_rules_keys() {
    use std::collections::HashMap;
    let seed = UserConfig {
        name: "Channel".into(),
        role: "user".into(),
        channel_bindings: HashMap::new(),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    // (1) empty after trim — confirms the existing branch still fires.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Channel/policy",
        Some(serde_json::json!({
            "channel_tool_rules": {
                "   ": { "allowed_tools": ["web_search"], "denied_tools": [] }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty: {body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("empty"),
        "error must mention empty channel name: {body:?}"
    );

    // (2) embedded newline — survives `trim()` but must be rejected.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Channel/policy",
        Some(serde_json::json!({
            "channel_tool_rules": {
                "foo\nbar": { "allowed_tools": ["web_search"], "denied_tools": [] }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "newline: {body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("control characters"),
        "error must mention control characters: {body:?}"
    );

    // (3) overlong key — adapter names are short slugs; cap at 64 chars.
    let long_name = "a".repeat(65);
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Channel/policy",
        Some(serde_json::json!({
            "channel_tool_rules": {
                long_name.clone(): { "allowed_tools": ["web_search"], "denied_tools": [] }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "long: {body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("longer than"),
        "error must mention length cap: {body:?}"
    );

    // (4) non-ASCII / spaces — must match the printable slug charset.
    for bad in ["telegram channel", "电报", "slack!"] {
        let (status, body) = json_request(
            &h,
            Method::PUT,
            "/api/users/Channel/policy",
            Some(serde_json::json!({
                "channel_tool_rules": {
                    bad: { "allowed_tools": ["web_search"], "denied_tools": [] }
                }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "charset {bad:?}: {body:?}");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or("")
                .contains("[a-zA-Z0-9_-]+"),
            "error must mention the slug charset for {bad:?}: {body:?}"
        );
    }

    // (5) valid `feishu` — sanity check that the new validators don't
    // over-reject real adapter names.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/Channel/policy",
        Some(serde_json::json!({
            "channel_tool_rules": {
                "feishu": { "allowed_tools": ["web_search"], "denied_tools": [] }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "valid feishu: {body:?}");
    assert_eq!(
        body["channel_tool_rules"]["feishu"]["allowed_tools"],
        serde_json::json!(["web_search"]),
        "valid rule must round-trip: {body:?}"
    );
}

/// Follow-up to PR #3229 — the user-list view must surface presence
/// flags so the dashboard can render "Policy / Memory / Budget" badges
/// without an extra round-trip per row. A bare user must report all
/// three flags as `false`; a customized user must report `true` for the
/// slots that are actually set.
#[tokio::test(flavor = "multi_thread")]
async fn users_list_summary_flags_reflect_policy_state() {
    use librefang_types::config::UserBudgetConfig;
    use librefang_types::user_policy::{UserMemoryAccess, UserToolPolicy};
    use std::collections::HashMap;

    let bare = UserConfig {
        name: "Bare".into(),
        role: "user".into(),
        ..Default::default()
    };
    let customized = UserConfig {
        name: "Custom".into(),
        role: "admin".into(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec![],
        }),
        memory_access: Some(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        }),
        budget: Some(UserBudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 10.0,
            max_monthly_usd: 100.0,
            ..Default::default()
        }),
        channel_tool_rules: HashMap::new(),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![bare, customized]).await;

    let (status, body) = json_request(&h, Method::GET, "/api/users", None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array");
    let bare_row = rows.iter().find(|r| r["name"] == "Bare").expect("Bare");
    let custom_row = rows.iter().find(|r| r["name"] == "Custom").expect("Custom");

    assert_eq!(bare_row["has_policy"], false, "{bare_row}");
    assert_eq!(bare_row["has_memory_access"], false, "{bare_row}");
    assert_eq!(bare_row["has_budget"], false, "{bare_row}");

    assert_eq!(custom_row["has_policy"], true, "{custom_row}");
    assert_eq!(custom_row["has_memory_access"], true, "{custom_row}");
    assert_eq!(custom_row["has_budget"], true, "{custom_row}");
}

/// Sanity check on the M3 follow-up — the list view's three summary
/// booleans must NOT be accompanied by the underlying contents. The
/// per-user detail endpoints (`/api/users/{name}/policy`, the budget
/// API) are the only paths that should expose policy bodies; leaking
/// `tool_policy` / `memory_access` / `budget` from the list would (a)
/// inflate response size proportionally to user count and (b) widen
/// the disclosure surface for callers that only have list-read.
#[tokio::test(flavor = "multi_thread")]
async fn users_list_summary_does_not_leak_policy_contents() {
    use librefang_types::config::UserBudgetConfig;
    use librefang_types::user_policy::{UserMemoryAccess, UserToolPolicy};
    use std::collections::HashMap;

    let seed = UserConfig {
        name: "Spy".into(),
        role: "user".into(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec!["shell_exec".into()],
        }),
        memory_access: Some(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: true,
            export_allowed: false,
            delete_allowed: false,
        }),
        budget: Some(UserBudgetConfig {
            max_hourly_usd: 0.5,
            max_daily_usd: 5.0,
            max_monthly_usd: 50.0,
            ..Default::default()
        }),
        channel_tool_rules: HashMap::new(),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, body) = json_request(&h, Method::GET, "/api/users", None).await;
    assert_eq!(status, StatusCode::OK);
    let row = body
        .as_array()
        .and_then(|a| a.iter().find(|r| r["name"] == "Spy"))
        .expect("row")
        .as_object()
        .expect("object");

    // The summary booleans must be present so the dashboard can render
    // badges. The actual policy bodies must NOT — they only belong on
    // the per-user detail endpoints.
    assert!(row.contains_key("has_policy"));
    assert!(row.contains_key("has_memory_access"));
    assert!(row.contains_key("has_budget"));
    assert!(
        !row.contains_key("tool_policy"),
        "tool_policy contents leaked into list view: {row:?}"
    );
    assert!(
        !row.contains_key("tool_categories"),
        "tool_categories leaked into list view: {row:?}"
    );
    assert!(
        !row.contains_key("memory_access"),
        "memory_access body leaked into list view: {row:?}"
    );
    assert!(
        !row.contains_key("channel_tool_rules"),
        "channel_tool_rules leaked into list view: {row:?}"
    );
    assert!(
        !row.contains_key("budget"),
        "budget body leaked into list view: {row:?}"
    );
    assert!(
        !row.contains_key("api_key_hash"),
        "api_key_hash leaked into list view: {row:?}"
    );
}

// ---------------------------------------------------------------------------
// API-key rotation (RBAC follow-up to #3054 / M3 / M6)
// ---------------------------------------------------------------------------

/// Seed a single user with an Argon2id hash of `plaintext_key` so the
/// auth-state snapshot the rotation endpoint mutates has something to swap.
fn seed_user_with_key(name: &str, plaintext_key: &str) -> UserConfig {
    let hash = librefang_api::password_hash::hash_password(plaintext_key)
        .expect("seed hash should succeed");
    UserConfig {
        name: name.to_string(),
        role: "admin".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: Some(hash),
        ..Default::default()
    }
}

/// Happy path — rotation returns a non-empty plaintext token in the response
/// body. The plaintext is the only place the new credential exists; if this
/// test ever asserts an empty string, operators have lost the rotated key
/// with no way to recover it.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_returns_new_plaintext() {
    let h = boot_with_seed_users(vec![seed_user_with_key("Alice", "old-plaintext")]).await;

    let (status, body) = json_request(&h, Method::POST, "/api/users/Alice/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK, "rotate failed: {body:?}");
    assert_eq!(body["status"], "ok");
    let new_key = body["new_api_key"].as_str().unwrap_or("");
    assert!(
        !new_key.is_empty(),
        "rotation must return a non-empty plaintext key: {body:?}"
    );
    assert_eq!(
        new_key.len(),
        64,
        "expected 32-byte hex (64 chars), got {}: {body:?}",
        new_key.len()
    );
    assert_eq!(body["sessions_invalidated"], 1);
}

/// Persisting the new hash is what closes the rotation loop — without it,
/// the next daemon restart would silently revive the old plaintext key.
/// We assert the hash on disk (and in the live kernel snapshot) is no
/// longer the seeded one.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_persists_new_hash() {
    let original_hash =
        librefang_api::password_hash::hash_password("old-plaintext").expect("seed hash");
    let seed = UserConfig {
        name: "Bob".to_string(),
        role: "admin".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: Some(original_hash.clone()),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, _) = json_request(&h, Method::POST, "/api/users/Bob/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK);

    let after = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "Bob")
        .cloned()
        .expect("user must still exist");
    let after_hash = after.api_key_hash.expect("rotation must leave a hash");
    assert_ne!(
        after_hash, original_hash,
        "api_key_hash unchanged after rotate — persist path is dead"
    );
    assert!(
        after_hash.starts_with("$argon2id$"),
        "post-rotation hash is not Argon2id PHC: {after_hash}"
    );
}

/// The actual session kill: `state.user_api_keys` is the in-memory snapshot
/// the auth middleware consults on every per-user bearer-token request.
/// After rotation, the OLD plaintext token must NOT verify against any
/// entry in the snapshot — the new hash has replaced it. This is the
/// integration point that distinguishes "rotated, but the leaked token
/// still authenticates until restart" from a real revocation.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_invalidates_existing_session() {
    let h = boot_with_seed_users(vec![seed_user_with_key("Carol", "carol-plaintext")]).await;

    // Pre-populate the live AppState snapshot from the seeded users so the
    // assertion below has something to invalidate. In the real server this
    // happens at boot via `configured_user_api_keys`; the unit harness
    // wires AppState directly without that step, so we mirror it here.
    {
        let cfg = h._state.kernel.config_ref();
        let initial = cfg
            .users
            .iter()
            .filter_map(|u| {
                let hash = u.api_key_hash.as_deref()?.trim();
                if hash.is_empty() {
                    return None;
                }
                Some(librefang_api::middleware::ApiUserAuth {
                    name: u.name.clone(),
                    role: librefang_kernel::auth::UserRole::from_str_role(&u.role),
                    api_key_hash: hash.to_string(),
                    user_id: librefang_types::agent::UserId::from_name(&u.name),
                })
            })
            .collect();
        *h._state.user_api_keys.write().await = initial;
    }

    // Sanity — the old plaintext verifies against the seeded hash before
    // rotation. If this assertion fails the test setup is wrong and the
    // post-rotation assertion below is meaningless.
    let pre_swap_old_token_verifies =
        h._state.user_api_keys.read().await.iter().any(|u| {
            librefang_api::password_hash::verify_password("carol-plaintext", &u.api_key_hash)
        });
    assert!(
        pre_swap_old_token_verifies,
        "test setup invariant — seeded hash should verify before rotation"
    );

    let (status, body) = json_request(&h, Method::POST, "/api/users/Carol/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK, "rotate failed: {body:?}");

    // The actual revocation. After rotation, NO entry in the live snapshot
    // verifies against the old plaintext — the in-memory state the auth
    // middleware reads from has been swapped, not just the on-disk file.
    let post_swap_old_token_verifies =
        h._state.user_api_keys.read().await.iter().any(|u| {
            librefang_api::password_hash::verify_password("carol-plaintext", &u.api_key_hash)
        });
    assert!(
        !post_swap_old_token_verifies,
        "old plaintext STILL authenticates against the live snapshot — \
         the rotate-key endpoint is not invalidating the in-memory user_api_keys, \
         which means a leaked token is only revocable by daemon restart"
    );

    // The new plaintext returned in the response must verify against the
    // freshly rotated hash — closes the loop end-to-end.
    let new_plaintext = body["new_api_key"].as_str().expect("plaintext in body");
    let new_token_verifies = h
        ._state
        .user_api_keys
        .read()
        .await
        .iter()
        .any(|u| librefang_api::password_hash::verify_password(new_plaintext, &u.api_key_hash));
    assert!(
        new_token_verifies,
        "new plaintext does not verify against the post-rotation snapshot — \
         the rotation surfaced a key that nobody can use"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_unknown_user_404() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::POST, "/api/users/Ghost/rotate-key", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got: {body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("not found"),
        "error must mention 'not found': {body:?}"
    );
}

/// Rotating MUST NOT zero out other per-user fields (RBAC M3 policy,
/// channel bindings, M5 budget). Mirrors the existing
/// `users_update_and_import_preserve_rbac_m3_policy_fields` test for the
/// rotation path — same regression risk.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_preserves_other_fields() {
    use librefang_types::user_policy::{UserMemoryAccess, UserToolPolicy};
    use std::collections::HashMap;

    let mut bindings = HashMap::new();
    bindings.insert("telegram".to_string(), "111".to_string());
    let seed = UserConfig {
        name: "Dan".into(),
        role: "viewer".into(),
        channel_bindings: bindings.clone(),
        api_key_hash: Some(librefang_api::password_hash::hash_password("seed-key").expect("seed")),
        budget: None,
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec!["shell_exec".into()],
        }),
        tool_categories: None,
        memory_access: Some(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        }),
        channel_tool_rules: HashMap::new(),
    };
    let h = boot_with_seed_users(vec![seed.clone()]).await;

    let (status, _) = json_request(&h, Method::POST, "/api/users/Dan/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK);

    let after = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "Dan")
        .cloned()
        .expect("user must still exist");
    assert_eq!(after.role, "viewer", "role flipped during rotate");
    assert_eq!(
        after.channel_bindings, bindings,
        "channel_bindings cleared during rotate"
    );
    assert_eq!(
        after.tool_policy, seed.tool_policy,
        "tool_policy cleared during rotate"
    );
    assert_eq!(
        after.memory_access, seed.memory_access,
        "memory_access cleared during rotate"
    );
}

/// Audit detail for a successful rotation must carry a short fingerprint
/// of the OLD `api_key_hash` so operators can later correlate this entry
/// with auth-failure log lines mentioning the same fingerprint.
///
/// We assert:
/// 1. Detail contains `(old: <8 hex chars>)`.
/// 2. The fingerprint matches `sha256(seeded_old_hash)[..4]` rendered as hex.
/// 3. The plaintext, the new hash, and the old hash itself never appear
///    in the audit detail — only the fingerprint.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_audit_includes_old_hash_fingerprint() {
    use sha2::{Digest, Sha256};

    let original_hash =
        librefang_api::password_hash::hash_password("erin-plaintext").expect("seed hash");
    let seed = UserConfig {
        name: "Erin".to_string(),
        role: "admin".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: Some(original_hash.clone()),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, body) = json_request(&h, Method::POST, "/api/users/Erin/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK, "rotate failed: {body:?}");

    let new_plaintext = body["new_api_key"]
        .as_str()
        .expect("plaintext in body")
        .to_string();

    // Compute the expected fingerprint the same way the handler does.
    let digest = Sha256::digest(original_hash.as_bytes());
    let mut expected_fp = String::with_capacity(8);
    for b in digest.iter().take(4) {
        expected_fp.push_str(&format!("{b:02x}"));
    }

    // Locate the rotation entry in the audit log. There may be additional
    // entries from the persist path (`users updated` ConfigChange), so we
    // filter by the `RoleChange` action and the user name.
    let entries = h._state.kernel.audit().recent(50);
    let rotate_entry = entries
        .iter()
        .find(|e| {
            matches!(e.action, librefang_runtime::audit::AuditAction::RoleChange)
                && e.detail.contains("api_key rotated")
                && e.detail.contains("for user Erin")
        })
        .unwrap_or_else(|| {
            panic!(
                "no rotate-key audit entry found among {} recent entries: {:#?}",
                entries.len(),
                entries
            )
        });

    assert!(
        rotate_entry
            .detail
            .contains(&format!("(old: {expected_fp})")),
        "audit detail missing fingerprint of OLD api_key_hash. \
         expected '(old: {expected_fp})' in: {detail:?}",
        detail = rotate_entry.detail
    );

    // Defensive — none of the secret material may leak into the audit detail.
    assert!(
        !rotate_entry.detail.contains(&new_plaintext),
        "audit detail leaked the new plaintext key: {detail}",
        detail = rotate_entry.detail
    );
    assert!(
        !rotate_entry.detail.contains(&original_hash),
        "audit detail leaked the full old api_key_hash (fingerprint only): {detail}",
        detail = rotate_entry.detail
    );
    assert!(
        !rotate_entry.detail.contains("erin-plaintext"),
        "audit detail leaked the OLD plaintext: {detail}",
        detail = rotate_entry.detail
    );
}

/// Rotating a user that previously had no `api_key_hash` (rare but
/// possible for a user added without a key, then granted one via rotate)
/// must still produce a parseable audit detail. We render the absent old
/// fingerprint as `(old: none)` so downstream parsers see a stable shape.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_audit_old_none_when_no_prior_hash() {
    let seed = UserConfig {
        name: "Frank".to_string(),
        role: "admin".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: None,
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]).await;

    let (status, _) = json_request(&h, Method::POST, "/api/users/Frank/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK);

    let entries = h._state.kernel.audit().recent(50);
    let rotate_entry = entries
        .iter()
        .find(|e| {
            matches!(e.action, librefang_runtime::audit::AuditAction::RoleChange)
                && e.detail.contains("for user Frank")
        })
        .expect("rotate-key audit entry must exist");

    assert!(
        rotate_entry.detail.contains("(old: none)"),
        "expected '(old: none)' for first-time key assignment, got: {detail}",
        detail = rotate_entry.detail
    );
}

/// Lock-ordering invariant: the `state.user_api_keys` snapshot must be
/// fully refreshed by the time the rotate-key handler returns 200 — i.e.,
/// no caller can observe a window where the on-disk `config.toml` carries
/// the new hash but the live `Vec<ApiUserAuth>` still verifies the old
/// plaintext. We can't deterministically reproduce the race in a unit
/// test (it depends on `reload_config` latency under contention), but we
/// CAN assert the post-condition that, immediately after the handler
/// resolves, the snapshot already reflects the on-disk state. If the
/// pre-fix ordering regressed (write → reload → swap, no lock held) this
/// assertion would still pass on a single-threaded happy path — but the
/// assertion below ALSO checks that the snapshot is refreshed even when
/// the caller acquires `user_api_keys.read()` immediately after the 200,
/// before any other request can race in. Treats the lock-held-across
/// invariant as a comment-anchored property; the deterministic regression
/// guard for the actual hash content is `users_rotate_key_invalidates_existing_session`.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_snapshot_consistent_with_disk_post_return() {
    let h = boot_with_seed_users(vec![seed_user_with_key("Gail", "gail-plaintext")]).await;

    // Mirror the daemon's boot-time wiring: `state.user_api_keys` starts
    // populated from the seeded `UserConfig`s.
    {
        let cfg = h._state.kernel.config_ref();
        let initial = cfg
            .users
            .iter()
            .filter_map(|u| {
                let hash = u.api_key_hash.as_deref()?.trim();
                if hash.is_empty() {
                    return None;
                }
                Some(librefang_api::middleware::ApiUserAuth {
                    name: u.name.clone(),
                    role: librefang_kernel::auth::UserRole::from_str_role(&u.role),
                    api_key_hash: hash.to_string(),
                    user_id: librefang_types::agent::UserId::from_name(&u.name),
                })
            })
            .collect();
        *h._state.user_api_keys.write().await = initial;
    }

    let (status, _) = json_request(&h, Method::POST, "/api/users/Gail/rotate-key", None).await;
    assert_eq!(status, StatusCode::OK);

    // After the handler returns, the on-disk hash and the live snapshot
    // for the rotated user must agree. Disagreement = the lock dropped
    // before the swap = the race window is back.
    let on_disk = h
        ._state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == "Gail")
        .and_then(|u| u.api_key_hash.clone())
        .expect("post-rotate Gail must have an on-disk hash");
    let in_memory = h
        ._state
        .user_api_keys
        .read()
        .await
        .iter()
        .find(|u| u.name == "Gail")
        .map(|u| u.api_key_hash.clone())
        .expect("post-rotate Gail must have an in-memory record");
    assert_eq!(
        on_disk, in_memory,
        "lock-ordering invariant broken: on-disk hash differs from live \
         user_api_keys snapshot immediately after rotate-key returned 200"
    );
}
