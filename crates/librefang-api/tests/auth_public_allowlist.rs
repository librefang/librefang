//! Static documentation test for the public-route catalog.
//!
//! LIMITATIONS (intentional, scope-of-fix):
//! - This test does NOT enumerate axum's live router. It walks a hardcoded
//!   REGISTERED_GET_ROUTES table maintained by hand.
//! - Adding a route in server.rs WITHOUT also adding it to REGISTERED_GET_ROUTES
//!   will silently pass this test. A proper router-enumeration drift gate is
//!   tracked as follow-up work; see #3712 acceptance criteria.
//!
//! What this test DOES catch:
//! - A path classified Authed in the table that's actually marked public via
//!   PUBLIC_ROUTES_* will fail.
//! - A path classified Public in the table that's actually still authed will
//!   fail (returns 401 vs expected non-401).
//! - Drift WITHIN the catalog: if a const slice's path doesn't match the
//!   table entry's classification, that's caught.
//!
//! Run: cargo test -p librefang-api --test auth_public_allowlist -- --nocapture

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::middleware::{
    PublicMatch, PublicMethod, PUBLIC_ROUTES_ALWAYS, PUBLIC_ROUTES_DASHBOARD_READS,
    PUBLIC_ROUTES_GET_ONLY,
};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    _state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

async fn boot_router_with_api_key(api_key: &str) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Populate the registry cache so the kernel boots without network access.
    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        _tmp: tmp,
        _state: state,
    }
}

/// Returns `true` if `path` is unconditionally public on GET requests — i.e.
/// it appears in `PUBLIC_ROUTES_ALWAYS`, `PUBLIC_ROUTES_GET_ONLY`, or is
/// handled by the dedicated `is_mcp_oauth_callback` guard in the middleware
/// (pattern: `/api/mcp/servers/*/auth/callback`).
fn is_in_always_public(path: &str) -> bool {
    for r in PUBLIC_ROUTES_ALWAYS {
        let ok = match r.match_kind {
            PublicMatch::Exact => path == r.path,
            PublicMatch::Prefix => path.starts_with(r.path),
        };
        if ok {
            return true;
        }
    }
    for r in PUBLIC_ROUTES_GET_ONLY {
        // Only check method=GetOnly entries (all of them in this slice are).
        if r.method != PublicMethod::GetOnly {
            continue;
        }
        let ok = match r.match_kind {
            PublicMatch::Exact => path == r.path,
            PublicMatch::Prefix => path.starts_with(r.path),
        };
        if ok {
            return true;
        }
    }
    // Mirror the is_mcp_oauth_callback guard: GET /api/mcp/servers/*/auth/callback
    // is public via a dedicated check in auth(), not via PUBLIC_ROUTES_GET_ONLY.
    if path.starts_with("/api/mcp/servers/") && path.ends_with("/auth/callback") {
        return true;
    }
    false
}

/// Returns `true` if `path` appears in `PUBLIC_ROUTES_DASHBOARD_READS`.
fn is_in_dashboard_reads(path: &str) -> bool {
    for r in PUBLIC_ROUTES_DASHBOARD_READS {
        let ok = match r.match_kind {
            PublicMatch::Exact => path == r.path,
            PublicMatch::Prefix => path.starts_with(r.path),
        };
        if ok {
            return true;
        }
    }
    false
}

async fn get_status(app: axum::Router, path: &str) -> StatusCode {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

// ---------------------------------------------------------------------------
// The route table under test.
//
// This is a hand-maintained list of GET-able paths. It is NOT auto-derived
// from server.rs — see the file-level LIMITATIONS note for why adding a
// route to server.rs without updating this table passes CI silently.
//
// Classification:
//   AlwaysPublic  -> must be in PUBLIC_ROUTES_ALWAYS or PUBLIC_ROUTES_GET_ONLY
//   DashboardRead -> must be in PUBLIC_ROUTES_DASHBOARD_READS
//   Authed        -> must NOT be in any public list (401 without token)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Route is unconditionally public (always returns non-401 even with auth configured).
    AlwaysPublic,
    /// Route is in the dashboard-reads group (public when require_auth_for_reads is off).
    DashboardRead,
    /// Route requires auth (must return 401 without a token when api_key is set).
    Authed,
}

struct RouteEntry {
    path: &'static str,
    expect: Expect,
}

const fn re(path: &'static str, expect: Expect) -> RouteEntry {
    RouteEntry { path, expect }
}

/// Every GET route registered in server.rs, classified by auth expectation.
///
/// Paths with dynamic segments use a representative concrete value.
const REGISTERED_GET_ROUTES: &[RouteEntry] = &[
    // Always-public (in PUBLIC_ROUTES_ALWAYS or PUBLIC_ROUTES_GET_ONLY)
    re("/", Expect::AlwaysPublic),
    re("/favicon.ico", Expect::AlwaysPublic),
    re("/logo.png", Expect::AlwaysPublic),
    re("/.well-known/agent.json", Expect::AlwaysPublic),
    re("/api/health", Expect::AlwaysPublic),
    re("/api/version", Expect::AlwaysPublic),
    re("/api/versions", Expect::AlwaysPublic),
    re("/api/auth/callback", Expect::AlwaysPublic),
    re("/api/auth/dashboard-login", Expect::AlwaysPublic),
    re("/api/auth/dashboard-check", Expect::AlwaysPublic),
    re("/api/auth/providers", Expect::AlwaysPublic),
    re("/api/auth/login", Expect::AlwaysPublic),
    re("/api/config/schema", Expect::AlwaysPublic),
    re("/api/pairing/complete", Expect::AlwaysPublic),
    re("/a2a/agents", Expect::AlwaysPublic),
    re("/dashboard/assets/main.js", Expect::AlwaysPublic),
    // PWA siblings of the dashboard shell. Mirror set in
    // `middleware.rs::PUBLIC_ROUTES_GET_ONLY`; source of truth for the file
    // list is `dashboard/public/`. Fetched by the browser before/around login
    // (manifest with `credentials="omit"` per spec, SW register, icons), so
    // they MUST stay public or every authenticated load 401-storms.
    re("/dashboard/icon-192.png", Expect::AlwaysPublic),
    re("/dashboard/icon-512.png", Expect::AlwaysPublic),
    re("/dashboard/manifest.json", Expect::AlwaysPublic),
    re("/dashboard/sw.js", Expect::AlwaysPublic),
    re("/locales/en.json", Expect::AlwaysPublic),
    re(
        "/api/providers/github-copilot/oauth/start",
        Expect::AlwaysPublic,
    ),
    re(
        "/api/mcp/servers/myserver/auth/callback",
        Expect::AlwaysPublic,
    ),
    re(
        "/api/mcp/servers/test-srv/auth/callback",
        Expect::AlwaysPublic,
    ),
    // Dashboard reads (public when require_auth_for_reads is off, which is default)
    re("/api/agents", Expect::DashboardRead),
    re("/api/a2a/agents", Expect::DashboardRead),
    re("/api/auto-dream/status", Expect::DashboardRead),
    re("/api/budget", Expect::DashboardRead),
    re("/api/budget/agents", Expect::DashboardRead),
    re("/api/budget/agents/some-agent-id", Expect::DashboardRead),
    // NOTE: /api/budget/agents/{id} is a prefix match in DASHBOARD_READS, so a
    // concrete path like the one above covers that case. No separate Authed
    // entry is needed.
    re("/api/channels", Expect::DashboardRead),
    re("/api/cron/list", Expect::DashboardRead),
    re("/api/hands", Expect::DashboardRead),
    re("/api/hands/active", Expect::DashboardRead),
    re("/api/hands/my-hand", Expect::DashboardRead),
    re("/api/mcp/catalog", Expect::DashboardRead),
    re("/api/mcp/health", Expect::DashboardRead),
    re("/api/config", Expect::DashboardRead),
    re("/api/mcp/servers", Expect::DashboardRead),
    re("/api/models", Expect::DashboardRead),
    re("/api/models/aliases", Expect::DashboardRead),
    re("/api/network/status", Expect::DashboardRead),
    re("/api/profiles", Expect::DashboardRead),
    re("/api/providers", Expect::DashboardRead),
    re("/api/sessions", Expect::DashboardRead),
    re("/api/skills", Expect::DashboardRead),
    re("/api/status", Expect::DashboardRead),
    re("/api/workflows", Expect::DashboardRead),
    // Auth-required endpoints (must 401 without Bearer token)
    re("/api/health/detail", Expect::Authed),
    // Security regression: /api/mcp/servers/{name} and /auth/status must NOT be
    // publicly reachable — the server config (including env vars) and OAuth token
    // state are sensitive. Only /auth/callback is public (via is_mcp_oauth_callback).
    re("/api/mcp/servers/test-srv", Expect::Authed),
    re("/api/mcp/servers/test-srv/auth/status", Expect::Authed),
    re("/api/agents/some-id/session", Expect::Authed),
    re("/api/agents/some-id/metrics", Expect::Authed),
    re("/api/agents/some-id/logs", Expect::Authed),
    re("/api/triggers", Expect::Authed),
    re("/api/logs/stream", Expect::Authed),
    re("/api/approvals", Expect::Authed),
    re("/api/approvals/audit", Expect::Authed),
    re("/api/approvals/totp/status", Expect::Authed),
    re("/api/users", Expect::Authed),
    re("/api/network/peers", Expect::Authed),
    re("/api/tools", Expect::Authed),
    re("/api/tools/file_read", Expect::Authed),
    re("/api/peers", Expect::Authed),
    re("/api/skills/my-skill", Expect::Authed),
    re("/api/a2a/tasks/some-task/status", Expect::Authed),
    re("/api/extensions", Expect::Authed),
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that every AlwaysPublic path exists in PUBLIC_ROUTES_ALWAYS or
/// PUBLIC_ROUTES_GET_ONLY — purely a catalog consistency check, no HTTP.
#[test]
fn always_public_paths_are_in_catalog() {
    let mut failures = Vec::new();
    for entry in REGISTERED_GET_ROUTES {
        if entry.expect != Expect::AlwaysPublic {
            continue;
        }
        if !is_in_always_public(entry.path) {
            failures.push(entry.path);
        }
    }
    if !failures.is_empty() {
        panic!(
            "These paths are marked AlwaysPublic in the test table but are \
             NOT listed in PUBLIC_ROUTES_ALWAYS or PUBLIC_ROUTES_GET_ONLY:\n  {}\n\n\
             Either add them to the middleware constants or change their Expect \
             classification in the test.",
            failures.join("\n  ")
        );
    }
}

/// Verify that every DashboardRead path exists in PUBLIC_ROUTES_DASHBOARD_READS.
#[test]
fn dashboard_read_paths_are_in_catalog() {
    let mut failures = Vec::new();
    for entry in REGISTERED_GET_ROUTES {
        if entry.expect != Expect::DashboardRead {
            continue;
        }
        if !is_in_dashboard_reads(entry.path) {
            failures.push(entry.path);
        }
    }
    if !failures.is_empty() {
        panic!(
            "These paths are marked DashboardRead in the test table but are \
             NOT listed in PUBLIC_ROUTES_DASHBOARD_READS:\n  {}\n\n\
             Either add them to the middleware constant or change their Expect \
             classification in the test.",
            failures.join("\n  ")
        );
    }
}

/// Verify that Authed paths do NOT appear in any public catalog.
#[test]
fn authed_paths_are_not_in_any_public_catalog() {
    let mut failures = Vec::new();
    for entry in REGISTERED_GET_ROUTES {
        if entry.expect != Expect::Authed {
            continue;
        }
        if is_in_always_public(entry.path) || is_in_dashboard_reads(entry.path) {
            failures.push(entry.path);
        }
    }
    if !failures.is_empty() {
        panic!(
            "These paths are marked Authed in the test table but appear in a \
             public catalog constant:\n  {}\n\n\
             This would silently bypass auth. Remove them from the middleware \
             constants or change their Expect classification to the correct \
             public tier.",
            failures.join("\n  ")
        );
    }
}

/// HTTP-level check: with an api_key configured, AlwaysPublic paths must NOT
/// return 401 (they may 200, 404, 405 — any non-401 is fine).
#[tokio::test(flavor = "multi_thread")]
async fn always_public_routes_are_reachable_without_token() {
    let harness = boot_router_with_api_key("test-secret-key").await;

    let mut failures = Vec::new();
    for entry in REGISTERED_GET_ROUTES {
        if entry.expect != Expect::AlwaysPublic {
            continue;
        }
        let status = get_status(harness.app.clone(), entry.path).await;
        if status == StatusCode::UNAUTHORIZED {
            failures.push(format!("{} -> {}", entry.path, status));
        }
    }

    if !failures.is_empty() {
        panic!(
            "AlwaysPublic routes returned 401 without a token (api_key IS configured):\n  {}\n\n\
             These paths must be reachable unauthenticated.",
            failures.join("\n  ")
        );
    }
}

/// HTTP-level check: with an api_key configured, Authed paths MUST return 401
/// when no Authorization header is sent.
#[tokio::test(flavor = "multi_thread")]
async fn authed_routes_require_token() {
    let harness = boot_router_with_api_key("test-secret-key").await;

    let mut failures = Vec::new();
    for entry in REGISTERED_GET_ROUTES {
        if entry.expect != Expect::Authed {
            continue;
        }
        // Skip duplicate entries (same path appears more than once in table).
        let status = get_status(harness.app.clone(), entry.path).await;
        if status != StatusCode::UNAUTHORIZED {
            failures.push(format!("{} -> {} (expected 401)", entry.path, status));
        }
    }

    if !failures.is_empty() {
        panic!(
            "Authed routes did NOT return 401 when called without a token:\n  {}\n\n\
             Either add them to a PUBLIC_ROUTES_* constant (if they should be \
             public) or fix the route registration / middleware ordering.",
            failures.join("\n  ")
        );
    }
}

/// HTTP-level check: with NO api_key configured (empty string), dashboard-read
/// paths must NOT return 401 (default: require_auth_for_reads = false).
#[tokio::test(flavor = "multi_thread")]
async fn dashboard_read_routes_reachable_without_auth_when_no_key_configured() {
    // Loopback + no api_key = open dev mode, so all endpoints pass through.
    // This is the baseline that confirms the dashboard can render without
    // credentials in the default local dev setup.
    let harness = boot_router_with_api_key("").await;

    let mut failures = Vec::new();
    for entry in REGISTERED_GET_ROUTES {
        if entry.expect != Expect::DashboardRead {
            continue;
        }
        let status = get_status(harness.app.clone(), entry.path).await;
        if status == StatusCode::UNAUTHORIZED {
            failures.push(format!("{} -> {}", entry.path, status));
        }
    }

    if !failures.is_empty() {
        panic!(
            "DashboardRead routes returned 401 in no-auth mode (no api_key set):\n  {}\n\n\
             In the default dev setup these paths must be reachable without credentials.",
            failures.join("\n  ")
        );
    }
}

/// Security regression: removing the broad `/api/mcp/servers/` prefix from
/// PUBLIC_ROUTES_GET_ONLY must keep the individual server config and auth-status
/// endpoints auth-protected, while the `/auth/callback` redirect path stays public.
///
/// Verifies fix for the regression introduced in the original allowlist PR where
/// `prefix_get("/api/mcp/servers/")` exposed `/api/mcp/servers/{name}` (returns
/// server config including env vars) and `/api/mcp/servers/{name}/auth/status`
/// (returns McpAuthState including OAuthTokens) without a Bearer token.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_servers_prefix_does_not_leak_protected_paths() {
    let harness = boot_router_with_api_key("test-secret-key").await;

    // These two must return 401 — they expose sensitive data.
    for path in [
        "/api/mcp/servers/test-srv",
        "/api/mcp/servers/test-srv/auth/status",
    ] {
        let status = get_status(harness.app.clone(), path).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} must return 401 without a token (exposes server config / OAuth tokens)"
        );
    }

    // The OAuth callback redirect must remain public — browser arrives here
    // without an API key after completing the OAuth provider flow.
    let callback_status = get_status(
        harness.app.clone(),
        "/api/mcp/servers/test-srv/auth/callback",
    )
    .await;
    assert_ne!(
        callback_status,
        StatusCode::UNAUTHORIZED,
        "/api/mcp/servers/test-srv/auth/callback must be reachable without a token (OAuth callback)"
    );
}
