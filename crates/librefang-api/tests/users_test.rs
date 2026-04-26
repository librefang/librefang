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
        provider_test_cache: dashmap::DashMap::new(),
        config_write_lock: tokio::sync::Mutex::new(()),
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
    assert_eq!(status, StatusCode::OK);

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
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("empty"),
        "error must mention the empty entry: {body:?}"
    );
}
