//! Integration tests for the audit-domain HTTP routes (refs #3571).
//!
//! Scope: only the audit slice. Other domains (budget/agents/skills/…) are
//! out of scope here.
//!
//! Routes covered (handlers in `src/routes/audit.rs` + `src/routes/system.rs`):
//!   - `GET /api/audit/query`   (admin-gated, in audit.rs)
//!   - `GET /api/audit/export`  (admin-gated, in audit.rs)
//!   - `GET /api/audit/recent`  (system.rs — currently uncovered)
//!   - `GET /api/audit/verify`  (system.rs — currently uncovered)
//!
//! What `api_integration_test.rs` already covers (intentionally NOT duplicated):
//!   - anon → 401 on `/audit/query`
//!   - viewer → 403 on `/audit/query`
//!   - admin → 200 baseline shape on `/audit/query`
//!   - `/audit/export?format=csv` Content-Type / Content-Disposition headers
//!
//! Gaps filled here:
//!   - `?action`/`?agent`/`?channel`/`?user` filtering returns the right rows
//!   - `?from` / `?to` malformed RFC-3339 → 400
//!   - `?limit` clamping (above MAX, below MIN)
//!   - `/audit/export?format=json` content shape (chunked array w/ `prev_hash`)
//!   - `/audit/export?format=bogus` → 400
//!   - `/audit/export` admin-gated (anon → 401, viewer → 403)
//!   - `/audit/recent` returns the canonical `PaginatedResponse` shape
//!     (`items`/`total`/`offset`/`limit`) plus `tip_hash`, and the `?n=` cap
//!     (capped at 1000 by handler)
//!   - `/audit/verify` returns `valid: true` on a fresh chain; `warning` field
//!     surfaces when the chain is empty

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use librefang_api::middleware;
use librefang_api::routes;
use librefang_kernel::audit::AuditAction;
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Test harness — an in-process Router with the audit + system routers wired
// behind the real auth middleware. Modeled after the helper in
// `api_integration_test.rs` (`start_test_server_with_rbac_users`) but exposes
// the Router directly so we drive it via `tower::oneshot` rather than a real
// TcpListener. That keeps the tests cheap to run in parallel and avoids any
// port contention.
// ---------------------------------------------------------------------------

struct AuditHarness {
    app: Router,
    state: Arc<routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for AuditHarness {
    fn drop(&mut self) {
        // Match the production server lifecycle: shut the kernel down so
        // background tasks stop and the in-memory SQLite handle closes.
        self.state.kernel.shutdown();
    }
}

/// Build a test router with RBAC users wired into both `KernelConfig.users`
/// and `AuthState.user_api_keys`. Each tuple is `(name, role, api_key)`.
fn build_audit_harness(api_key: &str, users: Vec<(&str, &str, &str)>) -> AuditHarness {
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

    let api_key_owned = api_key.to_string();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = api_key_owned;
        cfg.users = user_configs;
    }))
    .with_api_key(api_key)
    .with_user_api_keys(api_user_records);

    let (state, tmp, _cfg_path) = test.into_parts();

    let api_key_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        // Admin-gate is in-handler, so we still need anonymous requests to
        // flow through middleware when no api_key is set. With api_key
        // populated above, anonymous gets 401 at the middleware layer (which
        // is what the tests assert).
        allow_no_auth: true,
        audit_log: Some(state.kernel.audit().clone()),
    };

    let app = Router::new()
        .nest("/api", routes::audit::router())
        .nest("/api", routes::system::router())
        .layer(axum::middleware::from_fn_with_state(
            api_key_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    AuditHarness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Send a GET request through the router and return (status, body bytes).
async fn send_get(app: Router, path: &str, bearer: Option<&str>) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(Method::GET).uri(path);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = builder.body(Body::empty()).expect("build request");
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

fn body_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).expect("response body must be valid JSON")
}

/// Seed a few audit entries with mixed (user, agent, action, channel) so the
/// filter assertions can prove the right rows survive. We go through the
/// production `record_with_context` API rather than constructing `AuditEntry`
/// values directly so the hash-chain stays valid — `audit_verify` depends on
/// it.
fn seed_audit_entries(state: &routes::AppState) {
    let alice = UserId::from_name("Alice");
    let bob = UserId::from_name("Bob");
    let log = state.kernel.audit();
    log.record_with_context(
        "agent-alpha",
        AuditAction::ToolInvoke,
        "alpha used tool",
        "ok",
        Some(alice),
        Some("api".to_string()),
    );
    log.record_with_context(
        "agent-beta",
        AuditAction::ToolInvoke,
        "beta used tool",
        "ok",
        Some(bob),
        Some("telegram".to_string()),
    );
    log.record_with_context(
        "agent-alpha",
        AuditAction::PermissionDenied,
        "alpha denied",
        "denied",
        Some(alice),
        Some("api".to_string()),
    );
}

// ---------------------------------------------------------------------------
// /api/audit/query — filter / limit / time-range behaviour
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn audit_query_filters_by_action_case_insensitive() {
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(
        h.app.clone(),
        "/api/audit/query?action=permissiondenied",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let entries = body["items"].as_array().expect("items[]");
    assert!(
        !entries.is_empty(),
        "PermissionDenied filter must surface the seeded denial; got {body}"
    );
    for e in entries {
        assert_eq!(
            e["action"], "PermissionDenied",
            "every returned entry must match the action filter"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_query_filters_by_agent_and_channel() {
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(
        h.app.clone(),
        "/api/audit/query?agent=agent-beta&channel=telegram",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let entries = body["items"].as_array().expect("items[]");
    assert!(
        !entries.is_empty(),
        "agent+channel filter must match seeded beta entry"
    );
    for e in entries {
        assert_eq!(e["agent_id"], "agent-beta");
        assert_eq!(e["channel"], "telegram");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_query_filters_by_user_name_resolves_to_uuid() {
    // The handler accepts either the raw name or the stringified UUID for
    // `?user=`; pin the name path explicitly so a regression in
    // `user_matches_loose` shows up here.
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(
        h.app.clone(),
        "/api/audit/query?user=Bob",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let entries = body["items"].as_array().expect("items[]");
    assert!(
        !entries.is_empty(),
        "?user=Bob must match the Bob-attributed entry"
    );
    let bob = UserId::from_name("Bob").to_string();
    for e in entries {
        assert_eq!(e["user_id"], bob, "every returned entry must be Bob's");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_query_rejects_malformed_time_bounds_with_400() {
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);

    let (status, _) = send_get(
        h.app.clone(),
        "/api/audit/query?from=not-a-date",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "malformed RFC-3339 `from=` must be rejected with 400 (not silently dropped)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_query_clamps_oversized_limit() {
    // The handler clamps `limit` into `[1, MAX_AUDIT_QUERY_LIMIT]` (=5000).
    // A request asking for a million rows must come back reporting the
    // clamped value, not the requested one.
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(
        h.app.clone(),
        "/api/audit/query?limit=1000000",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let limit = body["limit"].as_u64().expect("limit must be a number");
    assert!(
        limit <= 5000,
        "limit must be clamped to MAX_AUDIT_QUERY_LIMIT (5000); got {limit}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_query_clamps_zero_limit_up_to_one() {
    // `?limit=0` → clamp(1, MAX). The response's `limit` field documents
    // what the handler actually applied.
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(
        h.app.clone(),
        "/api/audit/query?limit=0",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let limit = body["limit"].as_u64().expect("limit number");
    assert!(limit >= 1, "limit must be clamped up to >= 1; got {limit}");
    let entries = body["items"].as_array().expect("items[]");
    assert!(
        entries.len() <= limit as usize,
        "entries length ({}) must respect the reported limit ({})",
        entries.len(),
        limit
    );
}

// ---------------------------------------------------------------------------
// /api/audit/export — JSON body, unsupported format, admin gating
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn audit_export_json_body_is_array_with_prev_hash() {
    // The streaming JSON body is a chunked array. Each entry MUST carry
    // `prev_hash` so a downstream verifier can replay the SHA-256 chain off
    // the dump alone — the whole point of the chain. Pin that contract here
    // (the unit test in audit.rs covers `stream_json` directly; this one
    // covers it through the live route + middleware).
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(
        h.app.clone(),
        "/api/audit/export?format=json",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).expect("export must be valid JSON array");
    let arr = body.as_array().expect("export top-level must be an array");
    assert!(!arr.is_empty(), "JSON export must contain seeded entries");
    for e in arr {
        assert!(
            e.get("prev_hash").is_some(),
            "every JSON export entry must carry prev_hash for chain verification; got {e}"
        );
        assert!(e.get("hash").is_some(), "every entry must carry hash");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_export_default_format_is_json() {
    // No `?format=` → JSON. Catches a regression where the default is
    // changed silently.
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);
    seed_audit_entries(&h.state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/audit/export")
        .header("authorization", "Bearer alice-admin-key")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("application/json"),
        "default export format must be application/json; got {ct:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_export_unsupported_format_returns_400() {
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);

    let (status, _) = send_get(
        h.app.clone(),
        "/api/audit/export?format=xml",
        Some("alice-admin-key"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unsupported `format=` must return 400 (not silently fall through to JSON)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_export_rejects_anonymous() {
    // Same threat model as `/audit/query` — anonymous callers cannot be
    // allowed near the chain even on a no-auth deployment. With
    // `api_key` configured on the kernel, the middleware rejects with 401
    // before reaching the in-handler `require_admin` (which would otherwise
    // 403). Either way: no audit body for anon.
    let h = build_audit_harness("any-key", vec![("Alice", "admin", "alice-admin-key")]);

    let (status, _) = send_get(h.app.clone(), "/api/audit/export?format=csv", None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anonymous /api/audit/export must be rejected at the middleware (401)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_export_rejects_viewer() {
    let h = build_audit_harness(
        "any-key",
        vec![
            ("Alice", "admin", "alice-admin-key"),
            ("Eve", "viewer", "eve-viewer-key"),
        ],
    );

    let (status, _) = send_get(
        h.app.clone(),
        "/api/audit/export?format=csv",
        Some("eve-viewer-key"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer must be denied at the in-handler require_admin gate"
    );
}

// ---------------------------------------------------------------------------
// /api/audit/recent — currently uncovered by integration tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn audit_recent_returns_documented_shape() {
    // `/audit/recent` returns `PaginatedResponse{items,total,offset,limit}` plus `tip_hash`.
    let h = build_audit_harness("", vec![]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(h.app.clone(), "/api/audit/recent", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert!(body["items"].is_array(), "must carry items[]");
    assert!(body["total"].is_number(), "must carry total");
    assert!(body["offset"].is_number(), "must carry offset");
    assert!(body["limit"].is_number(), "must carry limit");
    assert!(
        body["tip_hash"].is_string(),
        "must carry tip_hash (chain-tip SHA-256)"
    );
    let total = body["total"].as_u64().unwrap();
    assert!(total >= 3, "seeded 3 entries; got total={total}");
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_recent_caps_n_at_1000() {
    // Handler clamps `?n=` at 1000. A megaroute request must not blow the
    // response size. We can't easily seed 1k entries in a unit test, so we
    // assert the structure stays valid at the cap and document the contract
    // (`n` is silently capped — there's no error).
    let h = build_audit_harness("", vec![]);

    let (status, bytes) = send_get(h.app.clone(), "/api/audit/recent?n=999999", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let entries = body["items"].as_array().expect("items[]");
    assert!(
        entries.len() <= 1000,
        "?n= must be capped at 1000; got {} entries",
        entries.len()
    );
}

// ---------------------------------------------------------------------------
// /api/audit/verify — currently uncovered by integration tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn audit_verify_reports_valid_on_fresh_chain() {
    let h = build_audit_harness("", vec![]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(h.app.clone(), "/api/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(
        body["valid"],
        serde_json::json!(true),
        "freshly-recorded entries must produce a valid chain; body={body}"
    );
    assert!(body["entries"].is_number(), "entries count must surface");
    assert!(body["tip_hash"].is_string(), "tip_hash must surface");
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_verify_omits_warning_when_chain_has_entries() {
    // The handler only attaches a `warning` field when the chain is empty
    // (forensic-value warning). With seeded entries we must NOT see that
    // field — pin both the presence-on-empty and absence-on-populated
    // contracts. We can't easily produce a 0-entry harness because the
    // kernel records its own startup events, so we only exercise the
    // populated path here; the empty-chain branch is covered by the
    // handler's own unit tests.
    let h = build_audit_harness("", vec![]);
    seed_audit_entries(&h.state);

    let (status, bytes) = send_get(h.app.clone(), "/api/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["valid"], serde_json::json!(true));
    let entries_count = body["entries"].as_u64().expect("entries number");
    assert!(entries_count > 0, "seeded chain must report >0 entries");
    assert!(
        body.get("warning").is_none(),
        "populated chain must NOT carry the empty-chain warning; body={body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_verify_surfaces_anchor_status_field() {
    // #3339 Tier-1: the verify response must surface anchor_status so the
    // dashboard can show "anchor: ok / diverged / none" alongside the
    // chain-validity badge. The audit harness wires an anchored AuditLog
    // by default (audit.anchor next to the temp DB), so on a clean chain
    // we expect anchor_enabled: true and anchor_status: "ok". We pin the
    // field names + happy-path value so a future refactor can't silently
    // drop the contract the dashboard relies on.
    let h = build_audit_harness("", vec![]);
    seed_audit_entries(&h.state);
    let (status, bytes) = send_get(h.app.clone(), "/api/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(
        body["anchor_enabled"],
        serde_json::json!(true),
        "harness wires audit.anchor next to the DB; body={body}"
    );
    assert_eq!(
        body["anchor_status"],
        serde_json::json!("ok"),
        "clean chain with anchor present must report 'ok'; body={body}"
    );
    assert!(
        body["anchor_path"].is_string(),
        "anchor_path must surface for the UI; body={body}"
    );
}
