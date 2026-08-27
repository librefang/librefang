//! Production middleware for the LibreFang API server.
//!
//! Provides:
//! - Request ID generation and propagation
//! - Per-endpoint structured request logging
//! - HTTP metrics recording (when telemetry feature is enabled)
//! - In-memory rate limiting (per IP)
//! - Accept-Language header parsing for i18n error responses

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::middleware::Next;
// Re-export `UserRole` through the api-layer auth boundary so that route
// modules (and tests) don't need to reach into `librefang_kernel::auth`
// directly. This keeps the `librefang-api` <-> `librefang-kernel` import
// surface narrow per issue #3744 — the underlying type still lives in the
// kernel; only the import path is centralized here.
pub use librefang_kernel::auth::UserRole;
use librefang_types::agent::UserId;
use librefang_types::i18n;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn, Instrument};

use librefang_telemetry::metrics;

/// The master API credential (#6613), as the auth middleware needs to see it.
///
/// Separate from `api_key_lock` because that lock holds a `\n`-joined list of literal strings to compare against, and the two members here do not fit that shape: a hash is verified rather than compared, and the transparent upgrade has to know which of the listed tokens is the master key rather than a derived dashboard session token — hashing the latter would write an upgrade hint for a credential the operator never configured.
///
/// Both string members are `RwLock` so `POST /api/config/reload` and the dashboard credential-change endpoint can swap them live, mirroring `api_key_lock`.
/// Write them through [`crate::server::refresh_master_credential`] so the composite lock and this struct can never disagree about what the master key is.
///
/// # Why the master key is hashed with SHA-256 and `dashboard_pass` with Argon2id
///
/// Both are credentials, both are verified rather than transmitted by the daemon, and they still want different hashes — do not "fix" one to match the other.
/// `dashboard_pass` is a human-chosen password: its entropy is low enough that an offline attacker who steals the hash can enumerate a dictionary, and a memory-hard KDF is precisely what makes that enumeration uneconomic.
/// The master `api_key` is a machine-generated bearer token, so there is no dictionary to enumerate; a slow KDF buys nothing against an offline attacker and charges the cost to every request instead.
///
/// That cost is not theoretical.
/// Argon2id verify is ~50–100 ms of CPU by construction, `api_key_hash` sits on the bearer path, and on a hash-only deployment *every* presented token reaches it — including every wrong one, from an unauthenticated caller, on a route with no login-attempt limiter.
/// A single connection replaying a garbage `Authorization` header would then consume a core continuously.
/// A fast constant-time hash over a high-entropy secret is the standard bearer-token construction, and it is already the one this codebase reached for: `password_hash::hash_device_token` hashes paired-device bearers with `$sha256$` and its doc comment makes the same argument for the same reason.
///
/// So the transparent upgrade writes `$sha256$`, `librefang hash-api-key` produces `$sha256$`, and the field docs on [`librefang_types::config::KernelConfig::api_key_hash`] recommend `$sha256$`.
/// `$argon2id$` stays *accepted* — an operator who deliberately uses a short human-memorable master key is better served by the KDF, and a hand-written or pre-#6613 value must keep working — but it is verified on a blocking thread, never inline, so the choice can never stall the async runtime.
/// See `crate::server::master_hash_matches` for that dispatch and [`crate::password_hash::is_cheap_to_verify`] for the predicate it keys on.
#[derive(Default)]
pub struct MasterKeyState {
    /// Resolved plaintext master key: `LIBREFANG_API_KEY`, else `vault:KEY`, else the literal `api_key`.
    /// Empty when the operator configured only a hash.
    /// Also present inside the `api_key_lock` composite, which is what the request path actually matches against; the copy here exists solely to identify a successful match as "the master key" for the upgrade.
    plaintext: tokio::sync::RwLock<String>,
    /// `KernelConfig.api_key_hash` — `$sha256$…` (recommended) or `$argon2id$…`, empty when the operator still keeps the master key as plaintext.
    hash: tokio::sync::RwLock<String>,
    /// Daemon home directory, where the transparent-upgrade hint file is written.
    /// `None` in test harnesses that build an `AuthState` without a daemon home; the hint is then skipped rather than landing in the process CWD.
    home_dir: Option<std::path::PathBuf>,
    /// Set once, the first time a plaintext-only master key authenticates, so the hint file is written at most once per process instead of on every authenticated request.
    upgrade_hint_started: std::sync::atomic::AtomicBool,
}

impl MasterKeyState {
    /// Build for a daemon rooted at `home_dir`, with credentials filled in by
    /// [`crate::server::refresh_master_credential`].
    pub fn new(home_dir: std::path::PathBuf) -> Self {
        Self {
            home_dir: Some(home_dir),
            ..Self::default()
        }
    }

    /// Current `api_key_hash`, already trimmed by the writer.
    pub async fn hash(&self) -> String {
        self.hash.read().await.clone()
    }

    /// Is a master credential configured, in either form?
    ///
    /// The live-handle counterpart of `server::MasterCredential::is_configured`, for a caller that has this struct rather than a config snapshot.
    /// Asks both members directly instead of testing `api_key_lock` for emptiness: that composite also carries the derived dashboard session token, so a non-empty composite does not by itself mean a *master* credential exists, and an empty one does not mean none does — a hash-only daemon lists no plaintext at all.
    /// Getting that backwards is the #6613 bug in miniature.
    pub async fn is_configured(&self) -> bool {
        !self.plaintext.read().await.trim().is_empty() || !self.hash.read().await.trim().is_empty()
    }

    /// Replace both members after boot, a config reload, or a credential
    /// change. Taken together under one call so a reader can never observe a
    /// new plaintext beside a stale hash.
    pub async fn set(&self, plaintext: String, hash: String) {
        let mut plaintext_guard = self.plaintext.write().await;
        let mut hash_guard = self.hash.write().await;
        *plaintext_guard = plaintext;
        *hash_guard = hash;
    }

    /// Same as [`set`](Self::set) for a caller that holds no runtime — a
    /// synchronous test-harness builder configuring state before any request
    /// can observe it. Panics if either lock is held, which under that
    /// contract means the harness handed the state out too early.
    pub fn set_blocking(&self, plaintext: String, hash: String) {
        let mut plaintext_guard = self
            .plaintext
            .try_write()
            .expect("master key plaintext lock should be uncontended during setup");
        let mut hash_guard = self
            .hash
            .try_write()
            .expect("master key hash lock should be uncontended during setup");
        *plaintext_guard = plaintext;
        *hash_guard = hash;
    }

    /// Constant-time test for "this presented token is the master plaintext
    /// key". Returns `false` when no plaintext key is configured, so a
    /// hash-only deployment never treats an empty string as a match.
    async fn is_master_plaintext(&self, token: &str) -> bool {
        use subtle::ConstantTimeEq;
        let configured = self.plaintext.read().await;
        if configured.is_empty() {
            return false;
        }
        configured.len() == token.len() && token.as_bytes().ct_eq(configured.as_bytes()).into()
    }

    /// Claim the one-shot upgrade-hint slot. Returns the home directory to
    /// write into on the single call that wins the race, `None` afterwards.
    fn claim_upgrade_hint(&self) -> Option<std::path::PathBuf> {
        let home_dir = self.home_dir.as_ref()?;
        self.upgrade_hint_started
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
            .then(|| home_dir.clone())
    }
}

/// Shared state for the auth middleware.
///
/// Combines the static API key(s) with the active session store so the
/// middleware can validate both legacy deterministic tokens and the new
/// randomly generated session tokens in a single pass.
#[derive(Clone)]
pub struct AuthState {
    /// Composite key string: multiple valid tokens separated by `\n`.
    pub api_key_lock: Arc<tokio::sync::RwLock<String>>,
    /// Hashed form of the master `api_key` plus its transparent-upgrade
    /// bookkeeping (#6613). Shared with `AppState` so a config reload can
    /// swap the hash without a daemon restart.
    pub master_key: Arc<MasterKeyState>,
    /// Active sessions issued by dashboard login, keyed by token string.
    pub active_sessions:
        Arc<tokio::sync::RwLock<HashMap<String, crate::password_hash::SessionToken>>>,
    /// Whether dashboard username/password auth is configured.
    pub dashboard_auth_enabled: bool,
    /// Optional per-user API-key hashes used for role-based API access.
    ///
    /// Wrapped in a `RwLock` (mirroring `api_key_lock`) so the rotate-key
    /// endpoint can swap the in-memory snapshot atomically. Without a live
    /// swap, a leaked per-user bearer token could only be revoked by
    /// restarting the daemon — defeating the point of rotation.
    pub user_api_keys: Arc<tokio::sync::RwLock<Vec<ApiUserAuth>>>,
    /// When `true` and an `api_key` is configured, GET endpoints that are
    /// otherwise on the dashboard public-read allowlist (agents, config,
    /// budget, sessions, approvals, hands, skills, workflows, …) are forced
    /// through bearer authentication. Static assets, OAuth entry points, and
    /// `/api/health*` remain public so the daemon stays probeable.
    pub require_auth_for_reads: bool,
    /// Set from `LIBREFANG_ALLOW_NO_AUTH=1` to permit running without an
    /// api_key on a non-loopback bind. Off by default so empty keys
    /// fail closed for LAN/public origins (see issue #1034 port).
    pub allow_no_auth: bool,
    /// RBAC M5: optional handle to the kernel's audit log so the
    /// middleware can record `PermissionDenied` events when a request is
    /// rejected by the role gate. Wrapped in `Option` because some test
    /// harnesses construct `AuthState` without a kernel attached.
    pub audit_log: Option<Arc<librefang_kernel::audit::AuditLog>>,
}

#[derive(Clone)]
pub struct ApiUserAuth {
    pub name: String,
    pub role: UserRole,
    pub api_key_hash: String,
    /// Stable LibreFang user id derived from `name` via [`UserId::from_name`].
    /// Pre-computed at config-load so the auth middleware does not need a
    /// kernel handle to identify the caller.
    pub user_id: UserId,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedApiUser {
    pub name: String,
    pub role: UserRole,
    /// Same id stored on [`ApiUserAuth`]; downstream handlers read this
    /// from request extensions to pass the caller through to kernel
    /// `authorize()` calls and into [`librefang_kernel::audit::AuditEntry`].
    pub user_id: UserId,
}

impl AuthenticatedApiUser {
    /// The [`Principal`] this caller acts for, for stamping ownership on what a request creates (#7744).
    ///
    /// `None` for the synthetic root credential — the master api key, a trusted loopback caller, or an `allow_no_auth` deployment, all of which are admitted as `name: "root"` with the fixed [`ROOT_API_KEY_USER_ID`] sentinel.
    /// That sentinel says "authentication is off or the caller holds the daemon's own key", not "a person called root"; it names no `[[users]]` entry, so recording it as an owner would write a principal that resolves to nothing and looks like a real one.
    /// Returning `None` here lets the caller fall through to `config.toml: default_owner`, which is the key an operator actually reaches for when they want unattributed writes attributed.
    pub fn owner_principal(&self) -> Option<librefang_types::principal::Principal> {
        if self.user_id.0 == ROOT_API_KEY_USER_ID {
            return None;
        }
        Some(librefang_types::principal::Principal::user(self.user_id))
    }
}

/// A LibreFang identity resolved from a validated OIDC ID token (#7744).
///
/// Produced by [`crate::oauth::oidc_auth_middleware`], which is the only place that holds both the cryptographically validated `IdTokenClaims` and the `[external_auth]` config the grant is derived from, and consumed by [`auth`] at the two points where a request would otherwise be rejected as unauthenticated.
///
/// The extension exists **only** when an operator declared `[external_auth.role_map]` and the caller's `roles` claim matched an entry in it, so its mere presence is the authorization decision; `role` is the level that decision produced, in the same `Viewer < User < Admin < Owner` vocabulary every other credential path uses.
/// Absent the extension nothing changes: a caller with no claims, with claims but no configured map, or with claims that map to nothing, takes exactly the path it took before this type existed.
#[derive(Clone, Debug)]
pub struct OidcRoleGrant {
    /// Display name for audit and attribution — the `email` claim when the provider issued one, otherwise the `sub`.
    pub name: String,
    /// Highest-privilege role the caller's claims mapped to.
    pub role: UserRole,
    /// `UserId::from_name(&name)`, so an OIDC caller resolves to the same id as a `[[users]]` entry declared under that name and inherits its per-user tool policy and budget.
    pub user_id: UserId,
}

/// The local `[[groups]]` an identity provider's claims placed this caller in, for the lifetime of one request (#7746).
///
/// Produced by [`crate::oauth::oidc_auth_middleware`] from `[external_auth.group_map]`, and read by handlers that need the caller's effective teams rather than their privilege level — `GET /api/authz/whoami` today, and any future check of `Principal::Group` ownership against the live caller.
///
/// **Ephemeral by design.** This never reaches `config.toml`. Membership is recomputed from the presented token on every request and dropped with it, so a revocation in the identity provider propagates when the caller's token expires, with no local cleanup and no stale row in operator-owned config. `ExternalAuthConfig::group_map` carries the full argument.
///
/// It is a separate extension from [`OidcRoleGrant`] rather than a field on it, because the two are independently configurable: an operator may declare `group_map` and no `role_map` — SSO identity for ownership and channel-binding roles, API privilege still on local keys — and folding membership into the role grant would silently disable it for exactly that deployment.
///
/// Presence carries no privilege. Membership confers group role strings and group-shaped ownership; it never contributes a [`UserRole`]. An IdP group called `owner` is a group called `owner`, nothing more.
#[derive(Clone, Debug)]
pub struct IdpGroupMembership {
    /// Names of declared `[[groups]]` entries the caller matched. Always a
    /// subset of the configured groups — `translate_oidc_groups` drops a map
    /// target that names no declared group — and a `BTreeSet` so the order is
    /// the same on every node and in every serialization (#3298).
    pub groups: std::collections::BTreeSet<String>,
}

/// Marks requests admitted solely by the explicitly trusted loopback/no-auth deployment mode.
/// Most routes treat these as Owner-equivalent for local single-user compatibility, while especially sensitive handlers (notably the audit ledger) can still require an actual credential.
#[derive(Clone, Copy, Debug)]
pub struct TrustedNoAuthCaller;

/// Endpoints that mutate kernel-wide configuration, user accounts, or
/// daemon lifecycle. `librefang_kernel::auth::Action::{ModifyConfig,
/// ManageUsers}` requires `UserRole::Owner` at the kernel layer; the
/// HTTP surface must agree, otherwise an Admin API key can change
/// configuration / rotate the bearer token / reload the daemon that a
/// Owner is responsible for.
/// True when the response log should demote a 4xx from WARN to DEBUG
/// because the (status, path) pair is a known-noisy false positive,
/// not a real signal worth alerting on.
///
/// Today the only case is **401 on `/api/metrics`**: the endpoint is
/// auth-gated and `getMetricsText` in the dashboard polls it every
/// 10 s from `useTelemetryMetrics`. Any client whose bearer expired
/// (or never had one — Prometheus scrapers, ad-hoc `curl` watchers)
/// produces a steady WARN stream that drowns out the real auth
/// signal the blanket-4xx-WARN was designed to surface.
///
/// `uri` is the raw `OriginalUri` string (with optional query). The
/// query is stripped before comparing so `/api/metrics?foo=bar`
/// still suppresses correctly.
fn is_noisy_metrics_unauth(status: u16, uri: &str) -> bool {
    status == 401 && uri.split('?').next().is_some_and(|p| p == "/api/metrics")
}

fn is_owner_only_write(method: &axum::http::Method, path: &str) -> bool {
    // Only non-GET methods are candidates — reads are handled separately.
    if *method == axum::http::Method::GET {
        return false;
    }
    // Exact-match list. These are the only routes the current codebase
    // exposes that cross the "Owner action" line; add here rather than
    // matching a prefix so a new Admin-write endpoint doesn't silently
    // get locked to Owner by accident.
    if matches!(
        path,
        "/api/config"
            | "/api/config/set"
            | "/api/config/reload"
            | "/api/auth/change-password"
            | "/api/shutdown"
            // #3621: TOTP enrollment is an Owner-equivalent action — a
            // confirmed enrollment hands the holder approve power for every
            // privileged tool call, so any non-Owner bearer token must not
            // be able to start, confirm, or revoke the enrollment.
            | "/api/approvals/totp/setup"
            | "/api/approvals/totp/confirm"
            | "/api/approvals/totp/revoke"
            // #5981: registering a passkey grants the holder a phishing-resistant
            // login credential — an Owner-equivalent action, mirroring TOTP
            // enrollment above. The two `authentication-*` siblings are public
            // (they mint a session, gated by the WebAuthn assertion itself).
            | "/api/auth/passkey/registration-options"
            | "/api/auth/passkey/registration-verify"
            // A2A discover registers a remote agent into the pending registry,
            // expanding the trust surface; restrict to Owner so non-Owner API keys
            // cannot fill the registry or stage impersonation attempts (#3483).
            | "/api/a2a/discover"
    ) {
        return true;
    }
    // `POST /api/hands/{hand_id}/install-deps` shells out to a package
    // manager whose argv is read straight from the HAND.toml an Admin can
    // write via `/api/registry/content/hand`. Even with the program
    // allowlist + flag denylist added in `routes::skills::install_hand_deps`,
    // the endpoint still spawns a process under the daemon UID, which is the
    // exact privilege Owner controls — restrict to Owner so an Admin role
    // (which is "config write" by design) cannot turn into "process spawn".
    // The matching `check-deps` sibling is a read-only readiness probe and
    // intentionally stays at Admin.
    if *method == axum::http::Method::POST
        && path.starts_with("/api/hands/")
        && path.ends_with("/install-deps")
    {
        return true;
    }
    // #5981: revoking a passkey (`DELETE /api/auth/passkey/credentials/{id}`)
    // removes a login credential — Owner-equivalent, matched by prefix because
    // of the `{id}` path segment. The sibling GET list stays at the generic
    // Admin-or-above read gate.
    if *method == axum::http::Method::DELETE && path.starts_with("/api/auth/passkey/credentials/") {
        return true;
    }
    // RBAC user-management surface (M6) — every mutating call under
    // `/api/users*` (create / replace / delete / bulk import) maps to
    // `Action::ManageUsers` in the kernel, which requires `Owner`. We
    // match by prefix because the path can be `/api/users`,
    // `/api/users/{name}`, or `/api/users/import`. GET is left to the
    // generic Admin-or-above gate so the dashboard's user list and
    // permission simulator stay usable for Admins.
    if path == "/api/users" || path.starts_with("/api/users/") {
        return true;
    }
    // Group management (#7745) sits on the same side of the line as user
    // management, and for a sharper reason: a group's `roles` list confers role
    // strings on every member. An Admin per-user API key that could reach
    // `POST /api/groups` would create a group carrying whatever role it likes,
    // add itself as a member, and self-promote — the same escalation the
    // `/api/users*` gate above exists to close, one indirection further out.
    // Prefix-matched because the path can be `/api/groups`,
    // `/api/groups/{name}`, or `/api/groups/{name}/members/{user}`. GET is left
    // to the generic Admin-or-above gate so the dashboard's group list and the
    // `/api/users/{name}/groups` reverse lookup stay usable for an Admin.
    if path == "/api/groups" || path.starts_with("/api/groups/") {
        return true;
    }
    // Adding / updating / deleting an MCP server persists a config entry that
    // `connect_mcp_servers()` immediately spawns — a stdio transport is a raw
    // `command` + `args` executed under the daemon UID. That is process spawn,
    // the exact privilege install-deps above is Owner-gated to protect, so an
    // Admin ("config write" by design) must not be able to reach it (finding
    // #3). Gate ONLY the config-mutation verbs: `POST /api/mcp/servers` (add)
    // and `PUT` / `DELETE /api/mcp/servers/{name}` (update / remove). GET
    // (list / detail) stays at the generic Admin-or-above read gate, and the
    // `{name}/reconnect|taint|auth/*` sub-resources — which do not introduce a
    // new spawn command — keep their existing Admin gate. The `{name}` target
    // is matched by requiring a single trailing segment with no further `/`,
    // so the deeper sub-resource paths are excluded.
    if *method == axum::http::Method::POST && path == "/api/mcp/servers" {
        return true;
    }
    if (*method == axum::http::Method::PUT || *method == axum::http::Method::DELETE)
        && path.starts_with("/api/mcp/servers/")
        && !path["/api/mcp/servers/".len()..].contains('/')
    {
        return true;
    }
    // #6631: every plugin route that can put plugin-controlled code on an execution path is Owner-only.
    // Same reasoning as `/api/hands/{id}/install-deps` above — Admin is "config write" by design and must not be able to turn that into "run attacker-supplied code as the daemon user".
    //
    // Deliberately NOT gated, and each for a reason:
    //   * every GET (list / detail / status / doctor / lint / env / registries, and the context-engine reads) — reads stay at the Admin gate.
    //   * `POST /api/plugins/uninstall` and `POST /api/plugins/{name}/disable` REMOVE code from the execution path.
    //     Gating them to Owner would stop an Admin from shutting a malicious plugin off during an incident, which makes the system less safe, not more.
    //   * `POST /api/plugins/scaffold` writes a template into the plugins dir and executes nothing.
    if *method == axum::http::Method::POST && plugin_route_executes_plugin_code(path) {
        return true;
    }
    false
}

/// Does this `POST /api/plugins/...` path let plugin-controlled code run?
///
/// Split out from `is_owner_only_write` so the set is enumerable in one place and directly unit-testable against the route list in `routes::plugins`.
fn plugin_route_executes_plugin_code(path: &str) -> bool {
    // Fetches an attacker-nominated git repo / registry entry into the plugins dir.
    // Nothing runs during the clone itself, but this is the step that introduces the code, and the issue names source installation explicitly.
    if path == "/api/plugins/install" {
        return true;
    }
    // Same fetch as `/install`, plus it resolves the dependency graph and installs every unresolved dependency too — the identical capability through a second top-level path, so it needs its own exact-match check rather than falling through the `{name}/<action>` split below (this path has no `{name}` segment).
    if path == "/api/plugins/install-with-deps" {
        return true;
    }
    // Batch form of the per-plugin `reload` action below: pre-warms one or more plugins by calling the same `plugin_manager::reload_plugin`, just without a `{name}` segment to match on.
    if path == "/api/plugins/prewarm" {
        return true;
    }
    // `POST /api/plugins/batch` dispatches on a body field (`{"operation": "...", "plugins": [...]}`) and accepts `enable` and `sign` — both Owner-only as per-plugin actions below.
    // The auth layer sees only method and path, so there is no way to gate the dangerous operations and admit the safe ones here: leaving the path open would let an Admin reach `enable` and `sign` through it, in bulk, and defeat those gates entirely.
    //
    // The cost is that an Admin loses the batch convenience for `disable` and `lint`.
    // That does not weaken incident response — the per-plugin `{name}/disable` stays at Admin precisely so a malicious plugin can be shut off, and doing that one at a time is still available.
    // Splitting the route by operation (or moving the role check into the handler, where the body is visible) would restore the convenience; that is a larger change than closing the bypass and is not required to close it.
    if path == "/api/plugins/batch" {
        return true;
    }
    let Some(rest) = path.strip_prefix("/api/plugins/") else {
        return false;
    };
    // Only `{name}/<action>` shapes below; `{name}` alone is a GET.
    let Some((_name, action)) = rest.split_once('/') else {
        return false;
    };
    matches!(
        action,
        // Runs npm / pip / bundler / composer, so package lifecycle scripts and build dependencies execute under the daemon UID.
        "install-deps"
            // Invokes a hook directly — the most direct execution path there is.
            | "test-hook"
            // Also invokes the hook directly, via `run_hook_json` in a loop — `runs` executions per request rather than one, so it is `test-hook` with a multiplier, not a measurement of something already running.
            | "benchmark"
            // Pulls new code over the existing plugin from registry or git.
            | "upgrade"
            // Puts the plugin's hooks back in the dispatch path, so the next matching event runs its code.
            | "enable"
            // Evicts the persistent hook subprocesses, so an edited script is picked up on the next call.
            // The reload itself executes nothing; it is how edited code goes live.
            | "reload"
            // Same underlying `reload_plugin` call as `reload` above, invoked to warm persistent hook subprocesses ahead of the first real call rather than in response to an edit.
            | "prewarm"
            // Recomputes and writes `[integrity]` hashes into plugin.toml.
            // Load-time verification (`plugin_manager`) rejects a hook whose hash no longer matches, so re-signing is what makes a tampered script loadable again — a trust assertion, not a read.
            | "sign"
    )
}

/// Minimum [`UserRole`] required for a privileged GET endpoint, or `None`
/// for an ordinary read.
///
/// Some routes are registered as `GET` (a WebSocket upgrade is an HTTP GET,
/// and tmux window listing is a GET) but perform a privileged, non-read
/// action once connected. [`user_role_allows_request`] otherwise treats every
/// GET as read-only and allows all roles, which would let a `Viewer` obtain
/// capabilities the RBAC model denies. This helper is consulted BEFORE that
/// blanket rule so the resolved role is checked against the real capability:
///
/// - `/api/terminal/ws` and `/api/terminal/windows[/…]` open / manage an
///   interactive PTY under the daemon UID — process spawn, an Admin action
///   (mirrors the install-deps boundary). A `Viewer`/`User` key must not get
///   a shell (finding #4).
/// - `/api/agents/{id}/ws` accepts inbound messages that drive a full agent
///   turn (LLM calls, tool execution, budget spend) — the same capability the
///   REST `POST /api/agents/{id}/message` grants, which requires `User`+. A
///   `Viewer` is read-only and must not drive turns over the WS either
///   (finding #11).
///
/// `path` must already be normalized (version prefix stripped, no trailing
/// slash); the id/window segments are concrete here (this runs on the request
/// URI, not the route template), so agent WS is matched by prefix + suffix.
fn min_role_for_privileged_get(path: &str) -> Option<UserRole> {
    if path == "/api/terminal/ws" || path.starts_with("/api/terminal/windows") {
        return Some(UserRole::Admin);
    }
    if path.starts_with("/api/agents/") && path.ends_with("/ws") {
        return Some(UserRole::User);
    }
    // `/api/config/export` returns the raw on-disk config.toml verbatim — including inline plaintext secrets (the master `api_key`, `network.shared_secret`, provider/channel credentials) that the sibling `GET /api/config` redacts.
    // It is a plain GET, so the blanket "GET is read-only" rule below would hand every authenticated role (Viewer / User / Admin) the unredacted secrets — and a leaked master api_key re-presents as `Owner`, a full privilege escalation.
    // Gate it to Owner, matching the Owner-only gating of the `/api/config[/set|/reload]` writes whose whole purpose is to keep the bearer token Owner-controlled.
    if path == "/api/config/export" {
        return Some(UserRole::Owner);
    }
    // `GET /api/users/{name}/provider-keys` lists the provider NAMES a user
    // has stored an upstream LLM key for (#6460 Follow-up B). No secret value
    // is returned, but the set of providers a user is configured against is
    // still account-management metadata, and the sibling PUT/DELETE writes on
    // the same resource are Owner-only via `is_owner_only_write`. Gate the
    // read to Owner too so an Admin cannot enumerate another user's provider
    // credential layout — matching the Owner posture of the whole
    // `/api/users*` management surface. Matched by prefix + suffix because
    // the `{name}` segment is concrete on the request URI.
    if path.starts_with("/api/users/") && path.ends_with("/provider-keys") {
        return Some(UserRole::Owner);
    }
    None
}

/// Whitelist check for per-user API-key access.
///
/// - `Owner`: full access.
/// - `Admin`: full access **except** Owner-only writes (see
///   [`is_owner_only_write`]) — kernel-wide config, user management,
///   daemon lifecycle, and the bearer-token change endpoint.
/// - `User`: GET everything + POST to a limited set of endpoints
///   (agent messages, clone, approval actions).
/// - `Viewer`: GET only.
/// - All other methods (`PUT`/`DELETE`/`PATCH`) require `Admin`+.
///
/// Exception: a few routes are registered as `GET` but perform a privileged,
/// non-read action on connect (WebSocket upgrades into a shell / agent turn,
/// tmux window management). [`min_role_for_privileged_get`] gates those by
/// role BEFORE the blanket GET rule, so "GET only" for `Viewer` still holds
/// for genuine reads but not for those upgrade endpoints.
///
/// The `path` must already be normalized (no trailing slash, version prefix
/// stripped) before calling this function.
fn user_role_allows_request(role: UserRole, method: &axum::http::Method, path: &str) -> bool {
    // Owner-only writes: even Admin cannot touch these.
    if is_owner_only_write(method, path) {
        return role >= UserRole::Owner;
    }

    // Privileged GET endpoints: a handful of routes are registered as `GET`
    // (WebSocket upgrades, tmux window management) yet perform a non-read
    // action on connect — spawn an interactive shell, drive an agent turn, or
    // manage PTYs. The blanket `*method == GET => true` rule below assumes
    // GET == read-only and would wave a Viewer straight through, so these must
    // be role-gated FIRST.
    if *method == axum::http::Method::GET {
        if let Some(min_role) = min_role_for_privileged_get(path) {
            return role >= min_role;
        }
    }

    if role >= UserRole::Admin || *method == axum::http::Method::GET {
        return true;
    }

    if role < UserRole::User {
        return false;
    }

    // User role: only specific POST endpoints are allowed.
    if *method == axum::http::Method::POST {
        let agent_message = path.starts_with("/api/agents/")
            && (path.ends_with("/message") || path.ends_with("/message/stream"));
        let agent_clone = path.starts_with("/api/agents/") && path.ends_with("/clone");
        // Anchor the approval suffixes to the `/api/approvals/` prefix. These
        // suffixes are meant only for tool-call approval actions; left
        // unanchored they also match `/api/skills/pending/{id}/approve` and
        // `/api/a2a/agents/{id}/approve`, neither of which has an in-handler
        // role check, so a User bearer could approve pending skills and trust
        // A2A agents (privilege escalation).
        let approval_action = path == "/api/approvals/batch"
            || (path.starts_with("/api/approvals/")
                && (path.ends_with("/approve")
                    || path.ends_with("/approve_all")
                    || path.ends_with("/reject")
                    || path.ends_with("/reject_all")
                    || path.ends_with("/modify")));
        return agent_message || agent_clone || approval_action;
    }

    false
}

/// Build the 403 response for an RBAC denial and record it in the audit log.
///
/// Shared by the session-token and per-user-API-key auth branches so both
/// enforce `user_role_allows_request` identically — previously only the
/// per-user-key branch gated, while a session token was trusted with no
/// role check (latent: all sessions are Owner today, but a future
/// SSO-mapped non-Owner session would have bypassed Owner-only writes).
fn rbac_denied_response(
    auth_state: &AuthState,
    method: &axum::http::Method,
    path: &str,
    role: UserRole,
    user_id: UserId,
    lang: &'static str,
) -> Response<Body> {
    if let Some(ref audit) = auth_state.audit_log {
        audit.record_with_context(
            "system",
            librefang_kernel::audit::AuditAction::PermissionDenied,
            format!("{method} {path}"),
            format!("role={role}"),
            Some(user_id),
            Some("api".to_string()),
        );
    }
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "application/json")
        .header("content-language", lang)
        .body(Body::from(
            serde_json::json!({
                "error": format!("Role '{role}' is not allowed to access this endpoint")
            })
            .to_string(),
        ))
        .unwrap_or_default()
}

/// What consulting the OIDC role grant produced at a point where [`auth`] was about to reject the request (#7744).
enum OidcOutcome {
    /// No [`OidcRoleGrant`] in extensions — the caller is not an OIDC principal, or holds claims the operator mapped to nothing.
    /// The rejection [`auth`] was about to perform stands, unchanged.
    NoGrant,
    /// The grant is good for this route; [`AuthenticatedApiUser`] has been inserted and the request should proceed.
    Admitted,
    /// The grant is real but its role may not reach this route.
    /// Carries the localized 403, already recorded in the audit log.
    Denied(Response<Body>),
}

/// Admit a request on a validated OIDC role grant, applying the same route/role gate every other credential path applies.
///
/// Called only from the two points in [`auth`] that were about to return 401 — the fail-closed branch for a deployment with no local credential configured, and the final fallthrough after every local credential missed.
/// Placing it there rather than higher up is what makes the whole feature additive: an OIDC bearer cannot displace, outrank, or demote an identity that some other credential already established, because by the time this runs no other credential matched.
///
/// It cannot in practice compete with one either.
/// The grant exists only when the `Authorization: Bearer` value was a JWT this daemon verified against a configured provider's JWKS, and a master `api_key` or a per-user key is an opaque secret rather than a signed token, so one header value cannot satisfy both.
fn apply_oidc_grant(
    request: &mut Request<Body>,
    auth_state: &AuthState,
    method: &axum::http::Method,
    path: &str,
) -> OidcOutcome {
    let Some(grant) = request.extensions().get::<OidcRoleGrant>().cloned() else {
        return OidcOutcome::NoGrant;
    };
    if !user_role_allows_request(grant.role, method, path) {
        let lang = request
            .extensions()
            .get::<RequestLanguage>()
            .map(|rl| rl.0)
            .unwrap_or(i18n::DEFAULT_LANGUAGE);
        return OidcOutcome::Denied(rbac_denied_response(
            auth_state,
            method,
            path,
            grant.role,
            grant.user_id,
            lang,
        ));
    }
    debug!(
        role = %grant.role,
        "admitting request on an OIDC role claim mapped by external_auth.role_map"
    );
    request.extensions_mut().insert(AuthenticatedApiUser {
        name: grant.name,
        role: grant.role,
        user_id: grant.user_id,
    });
    OidcOutcome::Admitted
}

/// Pull a caller-provided token from the standard locations the auth path
/// understands. Precedence (matches the non-loopback flow at `auth(...)`):
///   1. `Authorization: Bearer <x>`
///   2. `X-API-Key: <x>`
///   3. `Sec-WebSocket-Protocol: bearer.<x>` — WS upgrade fallback.
///      Browsers cannot set custom headers on the WebSocket handshake, so
///      the dashboard encodes the token as a sub-protocol entry that starts
///      with `bearer.`. Without this branch the auth middleware (which runs
///      before any WS handler) would 401-storm every dashboard ws (terminal,
///      chat, agent stream). The matching ws handler echoes the protocol
///      back via `WebSocketUpgrade::protocols(...)` so the browser accepts
///      the handshake — see `ws::ws_bearer_protocol`.
///
/// SECURITY: `?token=` query-string auth is intentionally NOT supported.
/// Query parameters appear in server access logs, browser history, and HTTP
/// Referer headers forwarded to third parties, making them unsuitable for
/// carrying credentials.
fn extract_request_token(request: &Request<Body>) -> Option<String> {
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    if bearer.is_some() {
        return bearer;
    }
    if let Some(key) = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
    {
        return Some(key.to_string());
    }
    // WebSocket upgrade: sub-protocol entry of the form `bearer.<token>`.
    // Multiple sub-protocols may be comma-separated; pick the first that
    // starts with `bearer.`.
    request
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(',')
                .map(str::trim)
                .find(|p| p.starts_with("bearer."))
                .and_then(|p| p.strip_prefix("bearer."))
                .map(str::to_string)
        })
}

/// File name of the transparent-upgrade hint, relative to the daemon home.
/// Named here so the writer and its test agree on one string.
pub(crate) const API_KEY_HINT_FILE: &str = "api-key-hash.upgrade-hint";

/// Offer the operator an `api_key_hash` for a master key that is still stored as plaintext (#6613).
///
/// Mirrors the `dashboard_pass` → `dashboard_pass_hash` path in `server.rs::dashboard_login`, including its central decision: the hash is written to a 0600 file and never logged, because the hash *is* the verifier — anyone who could read it out of the log stream could paste it into their own `config.toml` and authenticate.
///
/// Two differences from the dashboard path.
/// The hash is `$sha256$`, not Argon2id, because the master key is a machine-generated bearer rather than a human-chosen password and the resulting value is verified on every request — see the KDF section on [`MasterKeyState`] for the full argument.
/// And `claim_upgrade_hint` gates the whole thing to once per process, because API auth runs on every request while a dashboard login is rare, so the filesystem write must not repeat.
///
/// Returns the `spawn_blocking` handle so a test can await the write instead of polling for the file.
/// Production callers drop it: the hint is advisory, nothing downstream waits on it, and a failure is already reported through the `warn!` below.
/// `spawn_blocking` is for the *write* (temp file, fsync, rename), not for the hash — `hash_device_token` is a single SHA-256 and costs nothing.
#[must_use = "await the handle in tests; production callers should discard it explicitly"]
fn write_api_key_upgrade_hint(
    auth_state: &AuthState,
    plaintext_key: &str,
) -> Option<tokio::task::JoinHandle<()>> {
    let home_dir = auth_state.master_key.claim_upgrade_hint()?;
    let hash = crate::password_hash::hash_device_token(plaintext_key);
    Some(tokio::task::spawn_blocking(move || {
        let hint_path = home_dir.join(API_KEY_HINT_FILE);
        match crate::server::write_upgrade_hint(&hint_path, &hash, "api_key_hash", "api_key") {
            Ok(()) => info!(
                path = %hint_path.display(),
                "Master api_key authenticated as plaintext. An upgrade hash has been written to \
                 the file above (mode 0600). Persist it as `api_key_hash = \"<value>\"` in \
                 config.toml, remove `api_key`, then delete the hint file. Clients keep sending \
                 the same key — only the daemon's stored copy changes. The hash is unsalted \
                 SHA-256, which is the right verifier for a high-entropy generated key but not \
                 for a short memorable one: if your key is guessable, rotate it first with \
                 `librefang hash-api-key --generate` and use that hash instead."
            ),
            Err(e) => warn!(
                path = %hint_path.display(),
                error = %e,
                "Master api_key authenticated as plaintext but the upgrade-hint file could not \
                 be written. The hash is NOT logged — it is the verifier, and anyone with log \
                 access could authenticate with it. Fix the filesystem error and restart to \
                 regenerate."
            ),
        }
    }))
}

/// Request ID header name (standard).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Sentinel `user_id` for the synthetic Owner attribution applied when
/// the request authenticated only via the root `api_key` (operator's
/// master credential).
///
/// Chosen as a fixed UUID outside the `LIBREFANG_USER_NAMESPACE` v5
/// hash space so it cannot collide with a real `[users] name = "..."`
/// entry — even if an operator registers a user literally named `root`,
/// `UserId::from_name("root")` resolves to a *different* UUID and stays
/// isolated. Without this guarantee the master credential would silently
/// inherit any ACL / per-user budget cap configured for that user.
///
/// The specific bytes are arbitrary — what matters is that they're
/// stable across restarts (so audit-log queries can group by this id)
/// and unmistakable in `git log` / log output (`r00t…`).
pub const ROOT_API_KEY_USER_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-72006f0074a0");

/// Resolved language code extracted from the `Accept-Language` header.
///
/// Inserted into request extensions by the [`accept_language`] middleware so
/// that downstream route handlers can produce localized error messages.
#[derive(Clone, Debug)]
pub struct RequestLanguage(pub &'static str);

/// Per-request correlation id (#3639).
///
/// Inserted into [`Request::extensions_mut`] by [`request_logging`] **before**
/// the handler runs, so handlers can read the same id that ends up in the
/// `x-request-id` response header and the structured access-log line. Use
/// the [`crate::extractors::RequestId`] axum extractor on the handler side
/// — direct extension access is also supported.
#[derive(Clone, Debug)]
pub struct RequestIdExt(pub String);

/// Middleware: parse `Accept-Language` header and store the resolved language
/// in request extensions for downstream handlers.
///
/// Also sets the `Content-Language` response header to indicate which language
/// was used.
pub async fn accept_language(mut request: Request<Body>, next: Next) -> Response<Body> {
    let lang = request
        .headers()
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .map(i18n::parse_accept_language)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);

    request.extensions_mut().insert(RequestLanguage(lang));

    let mut response = next.run(request).await;

    if let Ok(header_val) = lang.parse() {
        response
            .headers_mut()
            .insert("content-language", header_val);
    }

    response
}

/// Middleware: inject a unique request ID and log the request/response.
///
/// The request_id is also published as a field on a per-request tracing
/// span that wraps the downstream handler.  Any child span opened inside
/// the handler — including the kernel orchestration spans and the
/// `llm.complete` / `llm.stream` spans on each LLM driver — inherits this
/// field automatically, so a single grep on `request_id=<uuid>` lights up
/// the full execution path (HTTP → kernel → LLM provider).  This closes
/// the propagation gap reported in #3775.
/// Prometheus `path` label used for every request that did not match a route.
///
/// A single constant so all unmatched (404 / fallback) requests share one
/// bounded series instead of one per distinct URI — see [`metric_path_label`]
/// and its call site in [`request_logging`].
const UNMATCHED_METRIC_PATH: &str = "<unmatched>";

/// Resolve the bounded Prometheus `path` label for a request.
///
/// `matched_path` is axum's route TEMPLATE (`Some("/api/agents/{id}")`) when
/// the router matched a route, or `None` for an unmatched request. Matched
/// routes keep their template (bounded cardinality); unmatched requests all
/// collapse to [`UNMATCHED_METRIC_PATH`]. The concrete request URI is
/// deliberately NOT a parameter: passing it on the unmatched branch is exactly
/// the unbounded-cardinality DoS this guards against.
fn metric_path_label(matched_path: Option<&str>) -> &str {
    matched_path.unwrap_or(UNMATCHED_METRIC_PATH)
}

pub async fn request_logging(mut request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let method = request.method().clone();
    let uri = request.uri().path().to_string();
    let start = Instant::now();

    // Prefer the matched route TEMPLATE (e.g. `/api/models/aliases/{alias}`)
    // for the metric `path` label so free-text route params never inflate
    // Prometheus label cardinality. `MatchedPath` is inserted by axum's
    // router before any `Router::layer` middleware runs, so it is present
    // for every request that matched a route. Fall back to `normalize_path`
    // on the concrete URI only when it is absent (e.g. 404 / unmatched),
    // which still collapses UUID/hex segments. Captured before `next.run`
    // consumes the request.
    let matched_path = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string());

    // #3639: stash the id in request extensions BEFORE the handler runs so
    // the [`crate::extractors::RequestId`] extractor (and any handler that
    // reads the extension directly) sees the same value that surfaces on
    // the response header and access-log span.
    request
        .extensions_mut()
        .insert(RequestIdExt(request_id.clone()));

    // Span wraps the entire downstream future so any `tracing::instrument`
    // (or manual span) opened inside the handler chain becomes a child of
    // this span and carries `request_id` for free.  `info_span!` (not
    // `debug_span!`) so the span is recorded at the default subscriber
    // level — debug-level spans get filtered out in release builds and
    // the propagation guarantee disappears with them.
    let request_span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %uri,
    );

    let mut response = next.run(request).instrument(request_span).await;

    let elapsed = start.elapsed();
    let status = response.status().as_u16();

    // Lift handler-resolved identifiers out of the response extensions and
    // onto the structured access-log line. Closes #3511 — without this,
    // tracing all requests for a specific agent/session across the kernel
    // boundary requires `RUST_LOG=debug` and string matching on raw URI
    // paths.
    let agent_id = response
        .extensions()
        .get::<crate::extensions::AgentIdField>()
        .map(|f| f.0.to_string());
    let agent_id_field = agent_id.as_deref().unwrap_or("");

    let session_id = response
        .extensions()
        .get::<crate::extensions::SessionIdField>()
        .map(|f| f.0.to_string());
    let session_id_field = session_id.as_deref().unwrap_or("");

    // 4xx/5xx elevated so auth storms and server faults surface; GET successes suppressed to avoid poll noise.
    if status >= 500 {
        error!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    } else if status >= 400 {
        // The blanket WARN-on-4xx surfaces auth storms and real client
        // bugs — but it also surfaces a known-noisy false positive:
        // unauthenticated polls of `/api/metrics`. The dashboard's
        // TelemetryPage refetches every 10s, and any client whose
        // bearer token expired (or who never logged in — Prometheus
        // scrapers, ad-hoc `curl` watchers) hammers a steady WARN
        // stream that drowns out the real auth signal we want to see.
        //
        // Demote that specific case to DEBUG. The endpoint returns
        // operational telemetry (uptime, agent counts, token usage —
        // see `routes/config.rs::prometheus_metrics`), so a 401 here
        // is "you don't have the token", not "you're attacking us".
        // Genuinely interesting 4xx on other paths still WARNs.
        if is_noisy_metrics_unauth(status, &uri) {
            debug!(
                request_id = %request_id,
                method = %method,
                path = %uri,
                status = status,
                latency_ms = elapsed.as_millis() as u64,
                agent_id = %agent_id_field,
                session_id = %session_id_field,
                "API request"
            );
        } else {
            warn!(
                request_id = %request_id,
                method = %method,
                path = %uri,
                status = status,
                latency_ms = elapsed.as_millis() as u64,
                agent_id = %agent_id_field,
                session_id = %session_id_field,
                "API request"
            );
        }
    } else if method == axum::http::Method::GET {
        debug!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    } else {
        info!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    }

    // Use the matched route template when available (bounded cardinality).
    // For UNMATCHED requests (404 / router fallback) `MatchedPath` is absent;
    // `metric_path_label` collapses those to a single constant sentinel rather
    // than the concrete URI. `normalize_path` only folds UUID/hex segments and
    // leaves arbitrary free-text verbatim, so an unauthenticated client
    // hitting `GET /nope-0000001`, `/nope-0000002`, … would otherwise mint a
    // new permanently retained Prometheus series per URI (the recorder has no
    // idle timeout) — an unbounded-cardinality memory-exhaustion DoS. The
    // helper takes only the matched template, so the raw URI can never reach
    // the label by construction.
    let metric_path = metric_path_label(matched_path.as_deref());
    metrics::record_http_request(metric_path, method.as_str(), status, elapsed);

    // Inject the request ID into the response header (always).
    if let Ok(header_val) = request_id.parse() {
        response.headers_mut().insert(REQUEST_ID_HEADER, header_val);
    }

    // #3639: stamp `request_id` (and a default `code` when missing) onto
    // every JSON 4xx/5xx response body so clients can correlate errors
    // with logs / support tickets without parsing the response header.
    // No-op for non-error responses, non-JSON bodies, and bodies that the
    // handler already populated with a `request_id`.
    if status >= 400 {
        response = normalize_json_error_body(response, &request_id).await;
    }

    response
}

/// Treat any `application/json` 4xx/5xx response with a `{"error": ...}`
/// body as the canonical error envelope and stamp `request_id` (#3639) plus
/// a default machine-readable `code` derived from the HTTP status when the
/// handler didn't already supply one. This centralises the contract so the
/// dozens of remaining `Json(json!({"error": "..."}))` sites in route
/// modules surface a uniform shape without per-site edits.
///
/// Bodies that fail to parse as a JSON object, or that are not JSON at all,
/// pass through untouched.
async fn normalize_json_error_body(response: Response<Body>, request_id: &str) -> Response<Body> {
    // Only touch JSON responses — leaving binary, HTML, plain-text, and
    // streaming bodies (SSE) alone is essential.
    let is_json = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !is_json {
        return response;
    }

    let status_code = response.status();
    let (mut parts, body) = response.into_parts();

    // Cap how much of the body we'll buffer to avoid OOM if a handler
    // somehow produced a multi-megabyte error response. 256 KiB is far above
    // any realistic error envelope. `axum::body::to_bytes` enforces the cap
    // for us — anything larger is left untouched.
    const MAX_ERROR_BODY_BYTES: usize = 256 * 1024;
    let bytes = match axum::body::to_bytes(body, MAX_ERROR_BODY_BYTES).await {
        Ok(b) => b,
        // Body too large or transport error — emit empty body to avoid
        // sending a half-buffered payload, but keep the original headers
        // so callers still see status + request_id header.
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    // Try parsing as a JSON object. Anything else (top-level array,
    // primitive, or invalid JSON) is left untouched.
    let mut value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
        _ => return Response::from_parts(parts, Body::from(bytes)),
    };
    let Some(obj) = value.as_object_mut() else {
        return Response::from_parts(parts, Body::from(bytes));
    };

    // Only stamp on bodies that look like our error envelope (have an
    // `"error"` key). Non-error JSON 4xx/5xx (rare but possible — e.g.
    // structured 422 with a custom shape) is passed through as-is.
    if !obj.contains_key("error") {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let mut mutated = false;
    let default_code = default_error_code_for_status(status_code);

    if !obj.contains_key("request_id") {
        obj.insert(
            "request_id".to_string(),
            serde_json::Value::String(request_id.to_string()),
        );
        mutated = true;
    }
    if !obj.contains_key("code") {
        obj.insert(
            "code".to_string(),
            serde_json::Value::String(default_code.to_string()),
        );
        // Mirror onto the legacy `type` alias so old clients see the same token.
        obj.entry("type")
            .or_insert(serde_json::Value::String(default_code.to_string()));
        mutated = true;
    }

    // #3639 deferred — also stamp into the nested `error` object when the
    // handler emitted the new envelope shape (`error: {code, message,
    // request_id}`). Ad-hoc `Json(json!({"error": "msg"}))` sites still
    // emit `error` as a string and are left untouched here; the flat
    // top-level fields above cover them.
    if let Some(err_obj) = obj.get_mut("error").and_then(|v| v.as_object_mut()) {
        if !err_obj.contains_key("request_id") {
            err_obj.insert(
                "request_id".to_string(),
                serde_json::Value::String(request_id.to_string()),
            );
            mutated = true;
        }
        if !err_obj.contains_key("code") {
            err_obj.insert(
                "code".to_string(),
                serde_json::Value::String(default_code.to_string()),
            );
            mutated = true;
        }
    }

    if !mutated {
        return Response::from_parts(parts, Body::from(bytes));
    }

    // Re-serialize. Failure here is essentially impossible (we just parsed
    // it), but fall back to the original bytes if it ever does.
    let new_bytes = match serde_json::to_vec(&value) {
        Ok(v) => v,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };
    // Update Content-Length so the framing stays correct.
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    if let Ok(len_val) = new_bytes.len().to_string().parse() {
        parts
            .headers
            .insert(axum::http::header::CONTENT_LENGTH, len_val);
    }
    Response::from_parts(parts, Body::from(new_bytes))
}

/// Map HTTP status code → default stable error code (#3639).
///
/// Only used when the handler didn't already supply a `code`. Values come
/// from [`librefang_types::error_code::ErrorCode`] so the alphabet stays in
/// one place.
fn default_error_code_for_status(status: StatusCode) -> &'static str {
    use librefang_types::error_code::ErrorCode;
    match status.as_u16() {
        400 => ErrorCode::BadRequest.as_str(),
        401 => ErrorCode::Unauthorized.as_str(),
        403 => ErrorCode::Forbidden.as_str(),
        404 => ErrorCode::NotFound.as_str(),
        409 => ErrorCode::Conflict.as_str(),
        422 => ErrorCode::InvalidInput.as_str(),
        429 => ErrorCode::RateLimited.as_str(),
        503 => ErrorCode::ServiceUnavailable.as_str(),
        s if s >= 500 => ErrorCode::InternalError.as_str(),
        _ => ErrorCode::BadRequest.as_str(),
    }
}

/// API version headers middleware.
///
/// Maximum JSON nesting depth accepted by the global request-body
/// guard. Defense-in-depth against deeply-nested
/// `[[[[…]]]]` payloads that would flow through the `Json<Value>`
/// extractors and recurse through downstream consumers (Cypher
/// conversion in memory routes, plugin config validators, etc.).
/// `serde_json` has no built-in depth cap, and the crate-level
/// `#![recursion_limit = "256"]` only applies to macro expansion —
/// it has no effect on runtime JSON deserialization. Audit:
/// check-json-depth-unused.
pub const MAX_JSON_BODY_DEPTH: usize = 32;

/// Tower middleware that enforces [`MAX_JSON_BODY_DEPTH`] on every
/// `application/json` request body before the handler sees it.
///
/// Non-JSON bodies pass through untouched. Empty bodies pass
/// through. A body whose `Content-Type` starts with
/// `application/json` is buffered (already capped by the global
/// `RequestBodyLimitLayer`), parsed once via `serde_json`, fed to
/// `crate::validation::check_json_depth`, and re-attached to the
/// request before forwarding. Buffering cost is paid only on JSON
/// requests; the body bytes round-trip with no copy beyond the
/// single `to_bytes` collect.
///
/// Audit: check-json-depth-unused.
pub async fn enforce_json_body_depth(request: Request<Body>, next: Next) -> Response<Body> {
    // Cheap pre-check: skip non-JSON content types and bail on
    // missing Content-Type. The audit only requires the guard for
    // `application/json` bodies; multipart uploads, plain text, raw
    // bytes etc. are unaffected.
    let is_json = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            let lower = s.trim().to_ascii_lowercase();
            // Match both `application/json` and `application/json;
            // charset=utf-8` style. Strict prefix check on the
            // media-type token only; never matches
            // `application/jsonpatch+json` or other vendor types
            // (those would need their own deserializer-specific
            // guards).
            lower == "application/json"
                || lower.starts_with("application/json;")
                || lower.starts_with("application/json ")
        })
        .unwrap_or(false);
    if !is_json {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    // `RequestBodyLimitLayer` upstream of this middleware already
    // caps the body size; the high ceiling here exists so a misordered
    // layer stack doesn't silently turn this into a memory bomb —
    // anything past it is rejected with 400 (which also short-circuits
    // a downstream OOM). 8 MiB matches the highest cap the kernel
    // currently exposes for `max_request_body_bytes`.
    const HARD_CEILING_BYTES: usize = 8 * 1024 * 1024;
    let bytes = match axum::body::to_bytes(body, HARD_CEILING_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "request body too large for JSON depth guard"})
                        .to_string(),
                ))
                .expect("static error response must build");
        }
    };
    // Empty body — nothing to validate; forward untouched.
    // Malformed JSON (`Err`) — forward as-is. The handler's own
    // deserializer will return a more specific 400 with the exact
    // column/offset, which is more useful to the client than a
    // generic depth-check error.
    if !bytes.is_empty() {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Err(e) = crate::validation::check_json_depth(&value, MAX_JSON_BODY_DEPTH) {
                // `ValidationError::into_response` formats the body
                // as the standard `ApiErrorResponse` shape; reuse it
                // so the response matches every other 4xx the API
                // surface returns.
                return axum::response::IntoResponse::into_response(e);
            }
        }
    }
    let request = Request::from_parts(parts, Body::from(bytes));
    next.run(request).await
}

/// Adds `X-API-Version` to every response so clients always know which version
/// they are talking to. When a request targets `/api/v1/...` the header reflects
/// `v1`; for the unversioned `/api/...` alias it returns the latest version.
///
/// Also performs content-type negotiation: if the `Accept` header contains
/// `application/vnd.librefang.<version>+json` the response version header
/// reflects the negotiated version. If the requested version is unknown the
/// server returns `406 Not Acceptable`.
pub async fn api_version_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let path = request.uri().path().to_string();

    let path_version = crate::versioning::version_from_path(&path);
    let accept_version = request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::versioning::version_from_accept_header);

    // Check Accept header for version negotiation
    let requested_accept_version = request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::versioning::requested_version_from_accept_header);

    // Validate negotiated version if provided
    if path_version.is_none() {
        if let Some(ver) = requested_accept_version {
            let known = crate::server::API_VERSIONS.iter().any(|(v, _)| *v == ver);
            if !known {
                return Response::builder()
                    .status(StatusCode::NOT_ACCEPTABLE)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "error": format!("Unsupported API version: {ver}"),
                            "available": crate::server::API_VERSIONS
                                .iter()
                                .map(|(v, _)| *v)
                                .collect::<Vec<_>>(),
                        })
                        .to_string(),
                    ))
                    .unwrap_or_default();
            }
        }
    }

    let mut response = next.run(request).await;

    // Determine the version to report. Explicit path versions win over headers.
    let version = if let Some(ver) = path_version {
        ver.to_string()
    } else if let Some(ver) = accept_version {
        ver.to_string()
    } else {
        crate::server::API_VERSION_LATEST.to_string()
    };

    if let Ok(val) = version.parse() {
        response.headers_mut().insert("x-api-version", val);
    } else {
        tracing::warn!("Failed to set X-API-Version header: {:?}", version);
    }

    response
}

// ---------------------------------------------------------------------------
// Public route catalog
//
// These typed constants are the single source of truth for which routes the
// auth middleware treats as publicly reachable.  They are intentionally
// `pub` so that integration tests can enumerate them and assert that every
// path either lives here or requires an Authorization header.
//
// Sorted alphabetically by path within each slice for deterministic ordering.
// ---------------------------------------------------------------------------

/// Whether a public route is reachable on any HTTP method or GET only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicMethod {
    /// Any HTTP method is public (no token required).
    Any,
    /// Only GET requests are public; other methods require auth.
    GetOnly,
}

/// Whether the path must match exactly or may be a prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicMatch {
    /// The normalised request path must equal `path` exactly.
    Exact,
    /// The normalised request path must start with `path`.
    Prefix,
}

/// A single entry in the public-route allowlist.
#[derive(Debug, Clone, Copy)]
pub struct PublicRoute {
    pub method: PublicMethod,
    pub path: &'static str,
    pub match_kind: PublicMatch,
}

impl PublicRoute {
    const fn exact_any(path: &'static str) -> Self {
        Self {
            method: PublicMethod::Any,
            path,
            match_kind: PublicMatch::Exact,
        }
    }
    const fn exact_get(path: &'static str) -> Self {
        Self {
            method: PublicMethod::GetOnly,
            path,
            match_kind: PublicMatch::Exact,
        }
    }
    // Kept available (no callers after `github-copilot/oauth/` moved
    // behind auth in audit `github-copilot-oauth-unauthenticated`) so
    // a future PublicRoute entry needing both prefix-match AND
    // any-method semantics doesn't have to re-derive the constructor.
    // Removing the constant would force the next public-prefix
    // operator to also rediscover the `PublicMethod::Any +
    // PublicMatch::Prefix` shape — a small but easy-to-fumble bit of
    // API design. `prefix_get` exists for the GET-only variant and is
    // currently the only used `PublicMatch::Prefix` arm.
    #[allow(dead_code)]
    const fn prefix_any(path: &'static str) -> Self {
        Self {
            method: PublicMethod::Any,
            path,
            match_kind: PublicMatch::Prefix,
        }
    }
    const fn prefix_get(path: &'static str) -> Self {
        Self {
            method: PublicMethod::GetOnly,
            path,
            match_kind: PublicMatch::Prefix,
        }
    }
}

/// Routes that are public on **any** HTTP method, regardless of auth config.
///
/// These are either static assets needed to render the login screen, auth
/// flow entry points, or minimal liveness probes that leak nothing sensitive.
///
/// Ordering note: entries here are grouped by semantic role (assets /
/// auth-flow / pairing / liveness / OAuth) rather than sorted alphabetically,
/// for readability. `PUBLIC_ROUTES_GET_ONLY` and `PUBLIC_ROUTES_DASHBOARD_READS`
/// are sorted alphabetically by path. Maintain the chosen ordering when adding
/// new entries to each slice.
pub const PUBLIC_ROUTES_ALWAYS: &[PublicRoute] = &[
    // Static assets / shell
    PublicRoute::exact_any("/"),
    PublicRoute::exact_any("/favicon.ico"),
    PublicRoute::exact_any("/logo.png"),
    // Auth flow entry points (method-free so POST also works)
    PublicRoute::exact_any("/api/auth/callback"),
    PublicRoute::exact_any("/api/auth/dashboard-check"),
    PublicRoute::exact_any("/api/auth/dashboard-login"),
    // Passkey login (#5981): both halves of the authentication ceremony run
    // before any session exists, so they must be public — exactly like
    // dashboard-login. The `registration-*` siblings are NOT here; they are
    // auth-gated Owner actions (see `is_owner_only_write`).
    PublicRoute::exact_any("/api/auth/passkey/authentication-options"),
    PublicRoute::exact_any("/api/auth/passkey/authentication-verify"),
    // Mobile pairing — phone has no API key yet
    PublicRoute::exact_any("/api/pairing/complete"),
    // Minimal liveness probes
    PublicRoute::exact_any("/api/health"),
    // Readiness probe (#6633). Must be public for the same reason
    // `/api/health` is: a Kubernetes `readinessProbe` is issued by the
    // kubelet, which holds no LibreFang credential, and a 401 would pin the
    // pod permanently out of Service endpoints. Its payload is check
    // names + coarse status only — no version, hostname, provider id, or
    // error text (see `routes::config::system::ready`); detailed
    // diagnostics stay behind the auth-gated `/api/health/detail`.
    PublicRoute::exact_any("/api/ready"),
    // NOTE: `/api/health/detail` is intentionally NOT public here. Its
    // payload includes `panic_count`, `restart_count`, `agent_count`,
    // embedding / extraction model ids, `config_warnings` from
    // `KernelConfig::validate()`, budget percentages, and LLM latency —
    // i.e. operational telemetry that should not be reachable from a
    // cold probe. The dashboard's `<OfflineBanner />` previously polled
    // this endpoint pre-auth and #4893 worked around the 401 spam by
    // exposing the detail payload publicly; the correct fix is for the
    // banner to poll the genuinely minimal `/api/health` instead, which
    // is what it does now. The middleware-internal comment block below
    // (covering the dashboard-read group) has long explained this
    // contract; this PR restores it (#4868 review).
    PublicRoute::exact_any("/api/version"),
    PublicRoute::exact_any("/api/versions"),
    // GitHub Copilot OAuth removed from the public-prefix list
    // (audit: github-copilot-oauth-unauthenticated).
    //
    // Pre-fix, both `POST /api/providers/github-copilot/oauth/start`
    // and `GET /api/providers/github-copilot/oauth/poll/{id}` were
    // public. A hostile pop-under page in a victim's browser could
    // POST to `http://localhost:4545/api/providers/.../oauth/start`
    // (simple POST → no preflight, no Origin check), display the
    // returned `user_code` + `verification_uri` from the daemon's
    // device-flow response in attacker-controlled UI (or
    // social-engineer the user to enter the code at
    // `github.com/login/device`), then poll until completion. The
    // poll handler then writes the attacker's GitHub Copilot
    // access token into `secrets.env` and the daemon environment
    // (`providers.rs:2220-2236`) — every subsequent outbound LLM
    // call routes through the attacker's GitHub account, billed
    // to them and observable by them.
    //
    // The dashboard already authenticates before initiating the
    // device flow; no legitimate unauthenticated caller exists.
    // Removing the public-prefix entry forces the standard auth
    // gate to apply.
];

/// Routes that are public on **GET only**, regardless of auth config.
pub const PUBLIC_ROUTES_GET_ONLY: &[PublicRoute] = &[
    PublicRoute::exact_get("/.well-known/agent.json"),
    // A2A: agent listing is public so external callers can discover agents
    // without a bearer token (A2A spec intent). All other /a2a/* paths require
    // auth (Bug #3781).
    PublicRoute::exact_get("/a2a/agents"),
    // `/api/auth/providers` is intentionally NOT here. Enumerating the
    // configured identity providers is information-gathering surface that
    // `require_auth_for_reads` exists to close, so it lives in
    // `PUBLIC_ROUTES_DASHBOARD_READS` below and is gated by that flag. The
    // handler returns names-only (id + display_name) to every caller and never
    // exposes the OAuth scope configuration (see `oauth::auth_providers`).
    // Auth login: exact for the base endpoint, prefix for the
    // provider-specific suffix `/api/auth/login/{provider}`. The
    // unsuffixed `prefix_get("/api/auth/login")` would have matched
    // any sibling that happened to share the prefix
    // (`/api/auth/login-status`, `/api/auth/loginhack`, etc.) and
    // silently leaked it as public — even though no such sibling
    // exists today (audit: login-prefix-match).
    PublicRoute::exact_get("/api/auth/login"),
    PublicRoute::prefix_get("/api/auth/login/"),
    // Config schema
    PublicRoute::exact_get("/api/config/schema"),
    // Dashboard assets (JS/CSS/fonts) — always public, SPA needs them for login page
    PublicRoute::prefix_get("/dashboard/assets/"),
    // PWA siblings of the dashboard shell — static bytes baked into the binary
    // via `include_dir!` (see `webchat.rs::resolve_dashboard_file`), identical
    // for every user and leaking nothing sensitive. They MUST be reachable
    // unauthenticated because:
    //   * the W3C App Manifest spec mandates `credentials="omit"` for
    //     `<link rel="manifest">` fetches absent `crossorigin="use-credentials"`,
    //     so the session cookie is intentionally not sent;
    //   * the service-worker register fetch and PWA icons are likewise issued
    //     before/around the login flow.
    // Without the exemption every authenticated dashboard load would log a
    // stream of WARN 401s for these paths.
    //
    // Source of truth for the asset set is `dashboard/public/` (bundled by
    // Vite into `dist/`). Adding a new PWA asset there means: (1) reference
    // it from `dashboard/index.html` (or `manifest.json`) and (2) add an
    // exact-match entry here. The rate limiter exempts the whole `/dashboard/`
    // tree via prefix in `rate_limiter.rs::is_rate_limit_exempt`, so no
    // change is needed there.
    PublicRoute::exact_get("/dashboard/icon-192.png"),
    PublicRoute::exact_get("/dashboard/icon-512.png"),
    PublicRoute::exact_get("/dashboard/manifest.json"),
    PublicRoute::exact_get("/dashboard/sw.js"),
    // i18n locale bundles — static, fetched before auth flow
    PublicRoute::prefix_get("/locales/"),
];

/// Routes in the "dashboard reads" group — public when `require_auth_for_reads`
/// is NOT enabled (or no auth is configured), authenticated otherwise.
///
/// All entries are GET-only. Prefix entries are marked `PublicMatch::Prefix`.
pub const PUBLIC_ROUTES_DASHBOARD_READS: &[PublicRoute] = &[
    PublicRoute::exact_get("/api/a2a/agents"),
    PublicRoute::exact_get("/api/agents"),
    // Provider enumeration: public only in open mode (no `require_auth_for_reads`).
    // The handler returns names-only (id + display_name) to every caller; the
    // IdP scope configuration is never exposed through this endpoint.
    PublicRoute::exact_get("/api/auth/providers"),
    PublicRoute::exact_get("/api/auto-dream/status"),
    PublicRoute::exact_get("/api/budget"),
    PublicRoute::exact_get("/api/budget/agents"),
    PublicRoute::prefix_get("/api/budget/agents/"),
    PublicRoute::exact_get("/api/channels"),
    PublicRoute::exact_get("/api/config"),
    // SECURITY #5139 (parity with #3367/#3941 for /api/approvals/*):
    // `/api/cron/` is intentionally absent. `GET /api/cron/jobs` and
    // `GET /api/cron/jobs/{id}` serialise the FULL `CronJob` — including the
    // user-authored prompt (`CronAction::AgentTurn.message` /
    // `SystemEvent.text`) and per-job `session_mode`. Leaving it in the
    // pre-auth dashboard-read group meant an operator who exposed 4545
    // remotely without `require_auth_for_reads = true` (the default) handed
    // every user-authored cron prompt to anyone reachable on the bind. The
    // dashboard attaches credentials on every request via its api helper, so
    // gating these reads is not a UX regression.
    PublicRoute::exact_get("/api/hands"),
    PublicRoute::exact_get("/api/hands/active"),
    PublicRoute::prefix_get("/api/hands/"),
    PublicRoute::exact_get("/api/mcp/catalog"),
    PublicRoute::exact_get("/api/mcp/health"),
    PublicRoute::exact_get("/api/mcp/servers"),
    PublicRoute::exact_get("/api/models"),
    PublicRoute::exact_get("/api/models/aliases"),
    PublicRoute::exact_get("/api/network/status"),
    PublicRoute::exact_get("/api/profiles"),
    PublicRoute::exact_get("/api/providers"),
    PublicRoute::exact_get("/api/sessions"),
    PublicRoute::exact_get("/api/skills"),
    PublicRoute::exact_get("/api/status"),
    PublicRoute::exact_get("/api/workflows"),
];

/// Check whether a normalised path matches a [`PublicRoute`] entry.
fn matches_route(route: &PublicRoute, path: &str, is_get: bool) -> bool {
    let method_ok = match route.method {
        PublicMethod::Any => true,
        PublicMethod::GetOnly => is_get,
    };
    if !method_ok {
        return false;
    }
    match route.match_kind {
        PublicMatch::Exact => path == route.path,
        PublicMatch::Prefix => path.starts_with(route.path),
    }
}

/// Bearer token authentication middleware.
///
/// When `api_key` is non-empty (after trimming), requests to non-public
/// endpoints must include `Authorization: Bearer <api_key>`.
/// If the key is empty or whitespace-only, auth is disabled entirely
/// (public/local development mode).
///
/// Also validates randomly generated session tokens from the active
/// session store, cleaning up expired sessions on each check.
pub async fn auth(
    axum::extract::State(auth_state): axum::extract::State<AuthState>,
    mut request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let api_key = auth_state.api_key_lock.read().await.clone();
    // Hashed master key (#6613). Snapshotted alongside `api_key` because a
    // hash-only deployment leaves `api_key_lock` empty yet is fully
    // authenticated — every "is auth configured" test below must consider
    // both, or an `api_key_hash`-only config falls through to the
    // fail-closed branch's loopback-Owner bypass.
    let master_key_hash = auth_state.master_key.hash().await;
    // Snapshot the per-user API key list once per request — `user_api_keys`
    // is now an `Arc<RwLock<Vec<…>>>` so the rotate-key endpoint can swap
    // entries live. The snapshot is cheap (small Vec of role records, no
    // hash work) and lets every downstream read avoid re-acquiring the
    // lock, including the constant-time `verify_password` loop below.
    let user_api_keys: Vec<ApiUserAuth> = auth_state.user_api_keys.read().await.clone();
    // SECURITY: Capture method early for method-aware public endpoint checks.
    let method = request.method().clone();

    // Shutdown is loopback-only (CLI on same machine) — skip token auth.
    // Normalize versioned paths: /api/v1/foo → /api/foo so public endpoint
    // checks work identically for both /api/ and /api/v1/ prefixes.
    let raw_path = request.uri().path().to_string();
    // Normalize: strip version prefix and trailing slashes so ACL checks
    // work consistently (e.g. "/api/v1/agents/" → "/api/agents").
    let after_version: String = if raw_path.starts_with("/api/v1/") {
        format!("/api{}", &raw_path[7..])
    } else if raw_path == "/api/v1" {
        "/api".to_string()
    } else {
        raw_path.clone()
    };
    // Strip a trailing slash for consistent ACL matching, but preserve the
    // root path "/" itself — otherwise stripping turns it into the empty
    // string, and `is_public` checks that compare against "/" (e.g. for the
    // dashboard HTML) silently miss, returning 401 for GET /.
    let path: &str = if after_version == "/" {
        "/"
    } else {
        after_version.strip_suffix('/').unwrap_or(&after_version)
    };
    // SECURITY: Loopback requests go through the same auth check as all other
    // connections. The unconditional loopback bypass has been removed — any
    // process on the same host must supply a valid token just like a remote
    // caller (see bug #3558).
    //
    // We still perform early token attribution here so that RBAC-gated
    // handlers (audit, per-user budget write, …) that require an
    // AuthenticatedApiUser extension work correctly for loopback callers that
    // carry a valid session or per-user API key (e.g. the CLI, a Vite
    // dev-proxy). After attribution the request falls through to the normal
    // is_public / token-verification flow below — there is no early return.
    {
        let is_loopback = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        if is_loopback {
            if let Some(token_str) = extract_request_token(&request) {
                // First try active dashboard sessions (random hex token exact
                // match) — the SPA proxied through Vite at 127.0.0.1 presents
                // a session cookie that must retain its role attribution.
                let session_attribution = {
                    let sessions = auth_state.active_sessions.read().await;
                    sessions.get(&token_str).cloned()
                };
                if let Some(session) = session_attribution {
                    if let (Some(name), Some(role_str)) = (session.user_name, session.user_role) {
                        let role = UserRole::from_str_role(&role_str);
                        let user_id = UserId::from_name(&name);
                        request.extensions_mut().insert(AuthenticatedApiUser {
                            name,
                            role,
                            user_id,
                        });
                    }
                    // Fall through to normal auth — the session token will be
                    // validated again in the main token-check path below.
                }
                // Try per-user API keys (Argon2 verify against api_key_hash).
                // Use the local `user_api_keys` snapshot taken at the top of
                // `auth()` — single source of truth for this request.
                else if let Some(user) = user_api_keys
                    .iter()
                    .find(|user| {
                        crate::password_hash::verify_password(&token_str, &user.api_key_hash)
                    })
                    .cloned()
                {
                    // Apply the role gate so a Viewer/User key on loopback
                    // cannot smuggle a write it would be denied over the LAN.
                    if !user_role_allows_request(user.role, &method, path) {
                        if let Some(ref audit) = auth_state.audit_log {
                            audit.record_with_context(
                                "system",
                                librefang_kernel::audit::AuditAction::PermissionDenied,
                                format!("{} {}", method, path),
                                format!("role={}", user.role),
                                Some(user.user_id),
                                Some("api".to_string()),
                            );
                        }
                        let lang = request
                            .extensions()
                            .get::<RequestLanguage>()
                            .map(|rl| rl.0)
                            .unwrap_or(i18n::DEFAULT_LANGUAGE);
                        return Response::builder()
                            .status(StatusCode::FORBIDDEN)
                            .header("content-type", "application/json")
                            .header("content-language", lang)
                            .body(Body::from(
                                serde_json::json!({
                                    "error": format!(
                                        "Role '{}' is not allowed to access this endpoint",
                                        user.role
                                    )
                                })
                                .to_string(),
                            ))
                            .unwrap_or_default();
                    }
                    request.extensions_mut().insert(AuthenticatedApiUser {
                        name: user.name,
                        role: user.role,
                        user_id: user.user_id,
                    });
                    // Fall through to normal auth — the token will be
                    // re-verified in the main token-check path below.
                }
            }
            // No early return — loopback requests continue through the
            // standard is_public check and token verification below.
        }
    }

    // Public endpoints that don't require auth (dashboard needs these).
    // SECURITY: /api/agents is GET-only (listing). POST (spawn) requires auth.
    // SECURITY: Public endpoints are GET-only unless explicitly noted.
    // POST/PUT/DELETE to any endpoint ALWAYS requires auth to prevent
    // unauthenticated writes (cron job creation, skill install, etc.).
    let is_get = method == axum::http::Method::GET;

    // "Always public" endpoints stay reachable with no token even when
    // `require_auth_for_reads` is on. These are either (a) static assets
    // needed to render the login screen, (b) auth flow entry points, or
    // (c) minimal liveness probes that leak nothing sensitive.
    //
    // `/api/status` intentionally stays out of this set: its handler returns
    // the full agent listing (id + name + model + profile) plus `home_dir`,
    // `api_listen`, and session count, which is exactly the enumeration
    // surface `require_auth_for_reads` exists to close. It lives in the
    // `dashboard_read_*` group below so it gets locked down with the flag.
    //
    // `/api/health/detail` is **not** in any public set — its own doc comment
    // at routes/config.rs:317 says it "requires auth", and it returns
    // `panic_count`, `restart_count`, `agent_count`, embedding/extraction
    // model IDs, `config_warnings` from `KernelConfig::validate()`, and the
    // event-bus drop count. All operational data that should not be reachable
    // from a cold probe. Unlike the dashboard read group, this endpoint
    // requires auth unconditionally regardless of `require_auth_for_reads`,
    // so the middleware contract finally matches the handler's own docs.
    // `/api/health` stays public because its payload is genuinely minimal
    // (status + version + a two-item checks array) and load balancers /
    // orchestrators need it for probing.
    // Walk PUBLIC_ROUTES_ALWAYS: public on any HTTP method regardless of auth config.
    let always_public_method_free = PUBLIC_ROUTES_ALWAYS
        .iter()
        .any(|r| matches_route(r, path, is_get));

    // MCP OAuth callback — browser redirect from OAuth provider, no API key.
    // Pattern: /api/mcp/servers/{name}/auth/callback — GET only.
    // This is the sole public entry point for the MCP OAuth flow; the prefix
    // "/api/mcp/servers/" is NOT in the PUBLIC_ROUTES_* slices so that
    // /api/mcp/servers/{name} and /auth/status remain auth-protected.
    let is_mcp_oauth_callback =
        is_get && path.starts_with("/api/mcp/servers/") && path.ends_with("/auth/callback");

    // Path has been trimmed of trailing slashes above, so `/dashboard/` is
    // normalized to `/dashboard`. Match the bare root as well as any
    // descendant so the login gate (and cookie session lookup below) don't
    // silently miss the root navigation.
    let is_dashboard_path = path == "/dashboard" || path.starts_with("/dashboard/");

    // Compute `auth_configured` early so we can decide whether the SPA
    // shell at `/dashboard/*` stays publicly reachable. When *any* form of
    // auth is configured, shell access goes behind the session cookie and
    // an unauthenticated browser gets a minimal inline login page
    // (see the 401 handler below). When no auth is configured the shell
    // stays public so the out-of-the-box dev experience still works.
    let auth_configured = !api_key.trim().is_empty()
        || !master_key_hash.is_empty()
        || !user_api_keys.is_empty()
        || auth_state.dashboard_auth_enabled;
    // The inline login page (`login_page.html`) only speaks username/password,
    // so only gate the shell when *that* mode is actually enabled. API-key-only
    // deployments keep a public shell so the SPA can load its own API-key
    // entry UI; the individual `/api/*` endpoints still require a Bearer
    // token, which is the real security boundary.
    //
    // Dashboard assets (JS/CSS/font chunks) and locale bundles are in
    // PUBLIC_ROUTES_GET_ONLY; the dashboard shell is conditionally public
    // based on dashboard_auth_enabled (handled below).
    let dashboard_shell_public = !auth_state.dashboard_auth_enabled && is_dashboard_path;

    // Walk PUBLIC_ROUTES_GET_ONLY: public on GET only regardless of auth config.
    // MCP OAuth callbacks are handled separately by is_mcp_oauth_callback above
    // (prefix + suffix check), not via a PUBLIC_ROUTES_GET_ONLY prefix entry.
    let always_public_get_only = is_get
        && (PUBLIC_ROUTES_GET_ONLY
            .iter()
            .any(|r| matches_route(r, path, is_get))
            || dashboard_shell_public);

    let always_public =
        always_public_method_free || always_public_get_only || is_mcp_oauth_callback;

    // "Dashboard reads" — the legacy public allowlist that lets the SPA
    // render before the user enters credentials. Downgraded to authenticated
    // when `require_auth_for_reads` is enabled AND an `api_key` is configured,
    // so a remote attacker can no longer enumerate agents, config, budget,
    // sessions, approvals, hands, skills, or workflows.
    //
    // SECURITY #3367 + post-merge audit of #3941: /api/approvals/* is
    // intentionally absent — every read path there exposes `action_summary`
    // (the pending shell command). The dashboard attaches credentials on every
    // request via its api helper, so this is not a UX regression.
    //
    // NOTE: /api/logs/stream (SSE) is also intentionally excluded — it
    // streams real-time audit/log events and must require auth the same way
    // every other sensitive read endpoint does. (#3593/#3680)
    let dashboard_read_public = is_get
        && PUBLIC_ROUTES_DASHBOARD_READS
            .iter()
            .any(|r| matches_route(r, path, is_get));

    let enforce_auth_on_reads = auth_state.require_auth_for_reads && auth_configured;

    let is_public = always_public || (dashboard_read_public && !enforce_auth_on_reads);

    if is_public {
        return next.run(request).await;
    }

    // If no API key configured (empty/whitespace) and no other auth method is
    // active, fail closed for any request that did NOT come from loopback —
    // unless the operator explicitly opted in via LIBREFANG_ALLOW_NO_AUTH=1.
    //
    // SECURITY: This closes the openfang #1034 hole where an empty api_key
    // bypassed auth for every origin (LAN/public), exposing agent config,
    // channel tokens, and LLM keys to anyone reachable on the bind address.
    // Loopback already short-circuits above for the single-user dev UX, so
    // reaching this branch means the caller is on the LAN/WAN.
    let api_key = api_key.trim();
    if api_key.is_empty()
        && master_key_hash.is_empty()
        && user_api_keys.is_empty()
        && !auth_state.dashboard_auth_enabled
    {
        // Re-check ConnectInfo defensively — if it is missing for any reason
        // we MUST treat the origin as non-loopback (fail closed, never open).
        let is_loopback = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        if is_loopback || auth_state.allow_no_auth {
            // No auth configured + trusted origin (loopback, or explicit
            // LIBREFANG_ALLOW_NO_AUTH opt-in) means a fully-trusted local
            // operator — the same trust level as the root master credential.
            // Attribute an Owner-equivalent user so RBAC-gated handlers (memory
            // write ACL, KV owner checks, …) treat the caller as privileged.
            // Without this, guard_for_user(None) downgrades to the anonymous
            // Viewer fallback and every POST/PUT/DELETE /api/memory* returns
            // 403 — breaking the default `librefang start` → `curl POST
            // /api/memory` workflow. Uses the same sentinel as the root-key
            // path below (ROOT_API_KEY_USER_ID → fail-open Owner-default ACL).
            request.extensions_mut().insert(AuthenticatedApiUser {
                name: "root".to_string(),
                role: UserRole::Owner,
                user_id: UserId(ROOT_API_KEY_USER_ID),
            });
            request.extensions_mut().insert(TrustedNoAuthCaller);
            return next.run(request).await;
        }
        // A deployment whose only configured credential is `[external_auth]`
        // lands here — no `api_key`, no `[[users]]`, no dashboard password —
        // and used to be told to set an api_key even though the caller had
        // just presented a token this daemon verified against the IdP's JWKS
        // (#7744). Consulted after the loopback/`allow_no_auth` branch above,
        // so a trusted local operator still gets Owner rather than whatever
        // their IdP claims happen to map to.
        match apply_oidc_grant(&mut request, &auth_state, &method, path) {
            OidcOutcome::Admitted => return next.run(request).await,
            OidcOutcome::Denied(resp) => return resp,
            OidcOutcome::NoGrant => {}
        }
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("www-authenticate", "Bearer")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "error": "API key required for non-loopback requests. Set api_key in config.toml, bind to 127.0.0.1, or set LIBREFANG_ALLOW_NO_AUTH=1 to opt out."
                })
                .to_string(),
            ))
            .unwrap_or_default();
    }

    // Check Authorization: Bearer <token> header, then fallback to X-API-Key,
    // then fallback to Sec-WebSocket-Protocol: bearer.<token> for WS upgrades.
    // Browsers cannot set custom headers on WebSocket handshakes, so the
    // dashboard encodes the session token as a sub-protocol entry — this must
    // be checked here for non-loopback connections (Docker bridge, LAN) where
    // the loopback fast-path above is not taken.
    let bearer_token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let api_token = bearer_token
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
        })
        .or_else(|| {
            // WS upgrade fallback: Sec-WebSocket-Protocol: bearer.<token>
            request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.split(',')
                        .map(str::trim)
                        .find(|p| p.starts_with("bearer."))
                        .and_then(|p| p.strip_prefix("bearer."))
                })
        });

    // Cookie-based session token — only accepted for SPA shell navigation
    // (`/dashboard/*`). API endpoints still require a Bearer/header token so
    // a cross-site request that auto-forwards the cookie cannot trigger a
    // write. Pair with `SameSite=Lax` on the Set-Cookie (issued by
    // `dashboard_login`) for the usual CSRF posture.
    let cookie_session_token = if is_dashboard_path {
        request
            .headers()
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|header| {
                header
                    .split(';')
                    .map(str::trim)
                    .find_map(|kv| kv.strip_prefix("librefang_session="))
                    .map(str::to_string)
            })
    } else {
        None
    };

    // SECURITY: Use constant-time comparison to prevent timing attacks.
    // `matches_master_token` splits the `\n`-joined composite and filters
    // empty candidates; the WS and terminal upgrade paths call the same
    // function so all three surfaces agree on what "matches the master
    // credential" means.
    let mut header_auth =
        api_token.map(|token| crate::server::matches_master_token(api_key, token));

    // Master-key hash (#6613). Only reached when the constant-time comparison
    // above missed, so a plaintext-configured deployment does no hash work.
    // With `api_key` empty and only `api_key_hash` set there is nothing to
    // match, so every authenticated request takes this path — which is why
    // `master_hash_matches` verifies the recommended `$sha256$` form inline
    // and pushes the ~50–100 ms `$argon2id$` form onto a blocking thread
    // rather than stalling this worker. See the KDF section on
    // `MasterKeyState`.
    if header_auth == Some(false) && !master_key_hash.is_empty() {
        if let Some(token_str) = api_token {
            if crate::server::master_hash_matches(&master_key_hash, token_str).await {
                header_auth = Some(true);
            }
        }
    }

    // SECURITY: ?token= query-string auth is deliberately NOT checked here.
    // Query parameters are written to server access logs, retained in browser
    // history, and forwarded in HTTP Referer headers to third parties. Tokens
    // must only arrive via Authorization: Bearer or X-API-Key headers, or via
    // the session cookie. WebSocket upgrades are the sole exception (browsers
    // cannot set custom headers on WebSocket connections); they authenticate
    // via crate::ws::ws_auth_token, which never passes through this middleware.

    // Accept if header auth matches a static API key or legacy token.
    //
    // The root `api_key` is the operator's master credential — attribute
    // the request as an Owner-equivalent `AuthenticatedApiUser` so RBAC-
    // gated handlers (memory write ACL, `assert_kv_owner_or_admin`, etc.)
    // treat root-key callers as fully-privileged. Without this attribution
    // root-key callers fall through as "trusted but anonymous", which the
    // memory namespace guard correctly downgrades to Viewer-equivalent
    // (no writes / no deletes / no exports) and breaks the obvious
    // `librefang start` → `curl POST /api/memory` workflow.
    //
    // The synthetic user_id is a fixed sentinel UUID (`ROOT_API_KEY_USER_ID`),
    // NOT `UserId::from_name("root")` — the latter would collide with a
    // real `[users] name = "root"` entry in `config.toml`, silently
    // granting the master credential whatever ACL / per-user budget cap
    // that user has configured. The sentinel falls outside the
    // `LIBREFANG_USER_NAMESPACE` v5 hash space, so `AuthManager.users.get`
    // returns None and the fail-open Owner-default ACL applies (matching
    // the documented "master credential" contract).
    if header_auth == Some(true) {
        // Transparent upgrade (#6613), mirroring `dashboard_pass` →
        // `dashboard_pass_hash`: when the master key authenticated as
        // plaintext and no `api_key_hash` is configured yet, derive one and
        // leave it in a 0600 hint file for the operator to paste into
        // config.toml. Gated on the token actually being the master key so a
        // dashboard session token — also carried in `api_key_lock` — never
        // gets hashed into an api_key upgrade hint.
        if master_key_hash.is_empty() {
            if let Some(token_str) = api_token {
                if auth_state.master_key.is_master_plaintext(token_str).await {
                    // Handle discarded on purpose: the hint is advisory, the
                    // request must not wait on a filesystem write, and a
                    // failure already surfaces as a `warn!` from inside.
                    drop(write_api_key_upgrade_hint(&auth_state, token_str));
                }
            }
        }
        request.extensions_mut().insert(AuthenticatedApiUser {
            name: "root".to_string(),
            role: UserRole::Owner,
            user_id: UserId(ROOT_API_KEY_USER_ID),
        });
        return next.run(request).await;
    }

    // Check the active session store for randomly generated dashboard tokens.
    // Also prune expired sessions opportunistically. Cookie token is only
    // consulted for `/dashboard/*` navigation (filtered upstream).
    let provided_token = api_token.or(cookie_session_token.as_deref());
    if let Some(token_str) = provided_token {
        let mut sessions = auth_state.active_sessions.write().await;
        // Remove expired sessions while we hold the lock
        sessions.retain(|_, st| {
            !crate::password_hash::is_token_expired(
                st,
                crate::password_hash::DEFAULT_SESSION_TTL_SECS,
            )
        });
        if let Some(session) = sessions.get(token_str).cloned() {
            drop(sessions);
            // If the session was issued by a credential flow that carried
            // identity (dashboard_login attaches `user_name` + `user_role`),
            // rebuild the AuthenticatedApiUser extension so RBAC-gated
            // handlers (audit/query, per-user budget writes) can see the
            // role. Legacy sessions persisted before attribution was added
            // load with both fields `None` and continue through as
            // trusted-anonymous — preserves the pre-fix behaviour for any
            // session sitting in `~/.librefang/sessions.json` from older
            // builds.
            if let (Some(name), Some(role_str)) = (session.user_name, session.user_role) {
                let role = UserRole::from_str_role(&role_str);
                let user_id = UserId::from_name(&name);
                // Enforce the same RBAC gate as the per-user-API-key branch:
                // a session's role must be allowed to reach this endpoint.
                if !user_role_allows_request(role, &method, path) {
                    let lang = request
                        .extensions()
                        .get::<RequestLanguage>()
                        .map(|rl| rl.0)
                        .unwrap_or(i18n::DEFAULT_LANGUAGE);
                    return rbac_denied_response(&auth_state, &method, path, role, user_id, lang);
                }
                request.extensions_mut().insert(AuthenticatedApiUser {
                    name,
                    role,
                    user_id,
                });
            }
            return next.run(request).await;
        }
        drop(sessions);

        if let Some(user) = user_api_keys
            .iter()
            .find(|user| crate::password_hash::verify_password(token_str, &user.api_key_hash))
            .cloned()
        {
            if !user_role_allows_request(user.role, &method, path) {
                // RBAC M5: `rbac_denied_response` surfaces the denial in the
                // hash-chained audit log (best-effort via the `audit_log`
                // handle injected into AuthState at server build time) and
                // returns the localized 403. Shared with the session branch.
                let lang = request
                    .extensions()
                    .get::<RequestLanguage>()
                    .map(|rl| rl.0)
                    .unwrap_or(i18n::DEFAULT_LANGUAGE);
                return rbac_denied_response(
                    &auth_state,
                    &method,
                    path,
                    user.role,
                    user.user_id,
                    lang,
                );
            }

            request.extensions_mut().insert(AuthenticatedApiUser {
                name: user.name,
                role: user.role,
                user_id: user.user_id,
            });
            return next.run(request).await;
        }
    }

    // Every local credential missed. Before rejecting, consult the OIDC role
    // grant (#7744): `oidc_auth_middleware` runs ahead of this layer and, when
    // the bearer token was a JWT it could verify and the operator declared
    // `[external_auth.role_map]`, left a resolved role in extensions that no
    // handler used to read.
    match apply_oidc_grant(&mut request, &auth_state, &method, path) {
        OidcOutcome::Admitted => return next.run(request).await,
        OidcOutcome::Denied(resp) => return resp,
        OidcOutcome::NoGrant => {}
    }

    // Determine error message: was a credential provided but wrong, or missing entirely?
    // Use the request language (set by accept_language middleware) for i18n.
    let lang = request
        .extensions()
        .get::<RequestLanguage>()
        .map(|rl| rl.0)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);
    let translator = i18n::ErrorTranslator::new(lang);

    let credential_provided = header_auth.is_some();
    let error_msg = if credential_provided {
        translator.t("api-error-auth-invalid-key")
    } else {
        translator.t("api-error-auth-missing-header")
    };

    // Browser navigation to `/dashboard/*` with no valid session — serve a
    // minimal self-contained login page instead of a JSON error, so the SPA
    // bundle (and whatever it imports) never reaches an unauthenticated
    // caller.
    if is_get && is_dashboard_path && auth_state.dashboard_auth_enabled {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "text/html; charset=utf-8")
            .header("cache-control", "no-store")
            .body(Body::from(LOGIN_PAGE_HTML))
            .unwrap_or_default();
    }

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", "Bearer")
        .header("content-language", lang)
        .body(Body::from(
            serde_json::json!({"error": error_msg}).to_string(),
        ))
        .unwrap_or_default()
}

const LOGIN_PAGE_HTML: &str = include_str!("login_page.html");
// If the inline script in login_page.html changes, recompute its script-src
// SHA-256 below. dashboard_login_page_script_is_allowed_by_csp_hash enforces it.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' 'sha256-TDA4xCzDRyoMM+fopfpKCyivlfu44tSPBzidGFvUgNM='; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://fonts.gstatic.com; img-src 'self' data: blob:; connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://localhost:* wss://127.0.0.1:*; font-src 'self' https://fonts.gstatic.com; media-src 'self' blob:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'";

/// Security headers middleware — applied to ALL API responses.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-xss-protection", "1; mode=block".parse().unwrap());
    // Dashboard JavaScript is served from self.
    // The exact hash permits only the static inline submit handler in login_page.html.
    // SECURITY: 'unsafe-eval' and script-src 'unsafe-inline' remain forbidden (#3732).
    // 'unsafe-inline' remains in style-src because the React/Vite bundle injects CSS-in-JS style tags at runtime.
    headers.insert(
        "content-security-policy",
        CONTENT_SECURITY_POLICY.parse().unwrap(),
    );
    headers.insert(
        "referrer-policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    headers.insert(
        "cache-control",
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    headers.insert(
        "strict-transport-security",
        "max-age=63072000; includeSubDomains".parse().unwrap(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    /// Build an `AuthState` for a daemon whose master key exists only as
    /// `api_key_hash` — `api_key_lock` stays empty because a hash carries no
    /// plaintext to compare against (#6613).
    ///
    /// `$sha256$` keeps `verify_password` on its cheap prefix-dispatch branch
    /// so these tests do not pay the ~100 ms Argon2id derivation.
    fn hash_only_auth_state(key: &str) -> AuthState {
        let master_key = MasterKeyState::default();
        master_key.set_blocking(String::new(), crate::password_hash::hash_device_token(key));
        AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            master_key: Arc::new(master_key),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        }
    }

    fn hash_only_app(key: &str) -> Router {
        Router::new()
            .route("/api/private", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                hash_only_auth_state(key),
                auth,
            ))
    }

    /// Issue a request carrying a **loopback** `ConnectInfo`.
    ///
    /// Injecting it is load-bearing, not incidental. The fail-closed branch
    /// reads `ConnectInfo` and treats its absence as non-loopback, so a request
    /// built without one is rejected on the missing-origin rule no matter what
    /// the auth configuration says — a test written that way passes even with
    /// the fix reverted and proves nothing. Presenting a loopback peer is what
    /// makes the unauthenticated-Owner bypass reachable, so the 401 below is
    /// attributable to `auth_configured` and nothing else.
    async fn get_private(app: Router, bearer: Option<&str>) -> StatusCode {
        let mut req = Request::builder().uri("/api/private");
        if let Some(token) = bearer {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                12345,
            ))));
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn hash_only_master_key_authenticates_the_matching_bearer() {
        assert_eq!(
            get_private(hash_only_app("s3cret-key"), Some("s3cret-key")).await,
            StatusCode::OK,
            "the key behind api_key_hash must authenticate even with api_key empty"
        );
    }

    #[tokio::test]
    async fn hash_only_master_key_rejects_a_wrong_bearer() {
        assert_eq!(
            get_private(hash_only_app("s3cret-key"), Some("wrong-key")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn hash_only_master_key_rejects_a_missing_bearer_from_loopback() {
        // The load-bearing case, and the one that motivated #6613's
        // `|| !master_key_hash.is_empty()` clauses. `api_key_lock` is empty on
        // a hash-only daemon, so without them `auth_configured` is false and
        // the request reaches the fail-closed branch — which, for a loopback
        // peer, hands out a full unauthenticated Owner session rather than
        // rejecting. Loopback is exactly the origin an operator considers
        // trusted for a *dev* daemon and decidedly not for one they deliberately
        // put an `api_key_hash` on.
        assert_eq!(
            get_private(hash_only_app("s3cret-key"), None).await,
            StatusCode::UNAUTHORIZED,
            "a hash-gated daemon must not fall back to the loopback Owner bypass"
        );
    }

    #[tokio::test]
    async fn hash_only_master_key_rejects_an_empty_bearer() {
        // `api_key_lock` holds "" and the composite splits on '\n'. Were the
        // empty candidate not filtered out, `ct_eq` over two empty slices
        // would authenticate `Authorization: Bearer ` as Owner.
        assert_eq!(
            get_private(hash_only_app("s3cret-key"), Some("")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    // ── Transparent api_key → api_key_hash upgrade (#6613) ────────────────
    //
    // Suggestion #1 of the issue, and the half an operator never asks for
    // explicitly: a plaintext deployment keeps working and is *offered* a hash
    // on first authentication. Everything about the offer is a contract with
    // whoever reads the hint file next, so all of it is pinned here — the path,
    // the mode, the two field names in the body, the once-per-process gate, and
    // the credential that must NOT trigger it.

    /// `AuthState` for a plaintext-configured daemon rooted at `home_dir`, the
    /// posture the upgrade fires from: `api_key` set, `api_key_hash` empty.
    fn plaintext_auth_state(key: &str, home_dir: &std::path::Path) -> AuthState {
        let master_key = MasterKeyState::new(home_dir.to_path_buf());
        master_key.set_blocking(key.to_string(), String::new());
        AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(key.to_string())),
            master_key: Arc::new(master_key),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        }
    }

    #[tokio::test]
    async fn upgrade_hint_names_both_config_fields_and_verifies_the_key() {
        let tmp = tempfile::tempdir().unwrap();
        let state = plaintext_auth_state("plain-master-key", tmp.path());

        write_api_key_upgrade_hint(&state, "plain-master-key")
            .expect("first call must claim the hint slot")
            .await
            .expect("hint writer must not panic");

        let hint_path = tmp.path().join(API_KEY_HINT_FILE);
        let body = std::fs::read_to_string(&hint_path).expect("hint file must exist");
        // Both field names, each as the whole backticked token the instructions
        // render — asserting on the bare substring `api_key` would be satisfied
        // by `api_key_hash` alone and would not notice the removal step going
        // missing, which is the half that leaves the plaintext key on disk.
        assert!(
            body.contains("`api_key_hash = "),
            "the operator has to be told which field to set: {body}"
        );
        assert!(
            body.contains("`api_key`"),
            "and which plaintext field to remove: {body}"
        );

        // The payload is a verifier for the key that authenticated, not merely
        // a plausible-looking string — pasting it into config.toml has to keep
        // the same clients working, which is the whole promise of a
        // *transparent* upgrade.
        let hash = body
            .lines()
            .find(|line| line.starts_with('$'))
            .expect("hint body must carry a PHC-style hash line");
        assert!(crate::password_hash::verify_password(
            "plain-master-key",
            hash
        ));
        assert!(!crate::password_hash::verify_password("other-key", hash));
    }

    /// The upgrade emits `$sha256$`, not Argon2id.
    ///
    /// Not a stylistic preference: `api_key_hash` is verified on the bearer path
    /// and, once the operator removes `api_key`, on *every* request including
    /// every wrong one from an unauthenticated caller. An Argon2id verify is
    /// ~50–100 ms of CPU by design, so writing that format here would hand the
    /// operator a remote CPU-exhaustion vector as the reward for following our
    /// own migration advice. See the KDF section on `MasterKeyState`.
    #[tokio::test]
    async fn upgrade_hint_writes_the_cheap_to_verify_hash_format() {
        let tmp = tempfile::tempdir().unwrap();
        let state = plaintext_auth_state("plain-master-key", tmp.path());
        write_api_key_upgrade_hint(&state, "plain-master-key")
            .expect("claim")
            .await
            .expect("write");

        let body = std::fs::read_to_string(tmp.path().join(API_KEY_HINT_FILE)).unwrap();
        let hash = body.lines().find(|line| line.starts_with('$')).unwrap();
        assert!(
            crate::password_hash::is_cheap_to_verify(hash),
            "the format we hand the operator must not put a KDF on the auth hot \
             path: {hash}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upgrade_hint_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let state = plaintext_auth_state("plain-master-key", tmp.path());
        write_api_key_upgrade_hint(&state, "plain-master-key")
            .expect("claim")
            .await
            .expect("write");

        let hint_path = tmp.path().join(API_KEY_HINT_FILE);
        let mode = std::fs::metadata(&hint_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the hint holds a verifier — anyone who can read it can paste it \
             into their own config.toml and authenticate"
        );
    }

    #[tokio::test]
    async fn upgrade_hint_is_written_at_most_once_per_process() {
        // API auth runs on every request. Without the one-shot gate a busy
        // daemon would rewrite this file thousands of times a minute.
        let tmp = tempfile::tempdir().unwrap();
        let state = plaintext_auth_state("plain-master-key", tmp.path());
        write_api_key_upgrade_hint(&state, "plain-master-key")
            .expect("first call claims the slot")
            .await
            .expect("write");
        assert!(
            write_api_key_upgrade_hint(&state, "plain-master-key").is_none(),
            "the second call must not claim the slot again"
        );
    }

    #[tokio::test]
    async fn upgrade_hint_is_skipped_without_a_daemon_home() {
        // `MasterKeyState::default()` has no home (test harnesses that build an
        // AuthState directly). Skipping beats writing the daemon's master-key
        // verifier into whatever the process CWD happens to be.
        let state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("plain-master-key".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        assert!(write_api_key_upgrade_hint(&state, "plain-master-key").is_none());
    }

    /// A dashboard session token also rides in the `api_key_lock` composite, so
    /// "this token authenticated" is not the same question as "this token is the
    /// master key". Hashing a session token into an `api_key_hash` hint would
    /// tell the operator to configure a master credential they never chose — and
    /// one that expires. `is_master_plaintext` is the gate that separates them.
    #[tokio::test]
    async fn a_dashboard_session_token_is_not_the_master_plaintext_key() {
        let tmp = tempfile::tempdir().unwrap();
        let state = plaintext_auth_state("plain-master-key", tmp.path());
        assert!(
            state
                .master_key
                .is_master_plaintext("plain-master-key")
                .await
        );
        assert!(!state.master_key.is_master_plaintext("session-token").await);
    }

    /// A hash-only daemon has no plaintext to identify, so nothing can match —
    /// including the empty string a bare `Authorization: Bearer ` produces.
    #[tokio::test]
    async fn hash_only_daemon_never_reports_a_master_plaintext_match() {
        let state = hash_only_auth_state("s3cret-key");
        assert!(!state.master_key.is_master_plaintext("").await);
        assert!(!state.master_key.is_master_plaintext("s3cret-key").await);
    }

    /// End-to-end through `auth`: a plaintext-configured daemon that
    /// authenticates a request leaves the hint behind without the caller having
    /// asked for anything. This is what "transparent" means, and it is the path
    /// an existing deployment actually takes on upgrade.
    #[tokio::test(flavor = "multi_thread")]
    async fn authenticating_with_a_plaintext_master_key_leaves_an_upgrade_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let state = plaintext_auth_state("plain-master-key", tmp.path());
        let app = Router::new()
            .route("/api/private", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state, auth));

        assert_eq!(
            get_private(app, Some("plain-master-key")).await,
            StatusCode::OK
        );

        // The write is spawned so the request never waits on the filesystem;
        // poll for the file rather than assuming a fixed delay is enough.
        let hint_path = tmp.path().join(API_KEY_HINT_FILE);
        for _ in 0..100 {
            if hint_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let body = std::fs::read_to_string(&hint_path)
            .expect("authenticating with a plaintext master key must leave a hint");
        assert!(body.contains("api_key_hash"));
    }

    /// The other side: once `api_key_hash` is configured, the migration is done
    /// and no hint should appear. Without this the daemon would keep re-offering
    /// an upgrade the operator already performed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hash_configured_daemon_writes_no_upgrade_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let master_key = MasterKeyState::new(tmp.path().to_path_buf());
        master_key.set_blocking(
            "plain-master-key".to_string(),
            crate::password_hash::hash_device_token("plain-master-key"),
        );
        let state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("plain-master-key".to_string())),
            master_key: Arc::new(master_key),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/private", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state, auth));

        assert_eq!(
            get_private(app, Some("plain-master-key")).await,
            StatusCode::OK
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !tmp.path().join(API_KEY_HINT_FILE).exists(),
            "an already-migrated daemon must not re-offer the upgrade"
        );
    }

    #[test]
    fn test_request_id_header_constant() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }

    #[test]
    fn is_noisy_metrics_unauth_matches_401_on_metrics_path() {
        // Bare path.
        assert!(is_noisy_metrics_unauth(401, "/api/metrics"));
        // With query string — Prometheus scrapers sometimes append
        // `?token=…` / `?format=…`; the suppression must still apply.
        assert!(is_noisy_metrics_unauth(401, "/api/metrics?token=xyz"));
        assert!(is_noisy_metrics_unauth(401, "/api/metrics?"));
    }

    #[test]
    fn is_noisy_metrics_unauth_rejects_other_statuses_and_paths() {
        // 403 / 404 / 500 etc. on /api/metrics keep WARNing — those
        // are real operational signals, not auth poll noise.
        assert!(!is_noisy_metrics_unauth(403, "/api/metrics"));
        assert!(!is_noisy_metrics_unauth(404, "/api/metrics"));
        assert!(!is_noisy_metrics_unauth(500, "/api/metrics"));
        assert!(!is_noisy_metrics_unauth(200, "/api/metrics"));
        // 401 on other paths must NOT be suppressed — those are the
        // genuine auth storms the blanket WARN was built to surface.
        assert!(!is_noisy_metrics_unauth(401, "/api/agents"));
        assert!(!is_noisy_metrics_unauth(401, "/api/config/reload"));
        assert!(!is_noisy_metrics_unauth(401, "/api/admin/shutdown"));
        // Prefix-only matches must not slip through — `/api/metrics2`,
        // `/api/metrics/foo`, etc. are different endpoints (or future
        // sub-paths).
        assert!(!is_noisy_metrics_unauth(401, "/api/metrics2"));
        assert!(!is_noisy_metrics_unauth(401, "/api/metrics/foo"));
        // Empty / nonsense paths don't match.
        assert!(!is_noisy_metrics_unauth(401, ""));
        assert!(!is_noisy_metrics_unauth(401, "/"));
    }

    /// Review-followup #1: the synthetic root user_id used for root-api_key
    /// callers MUST live outside `LIBREFANG_USER_NAMESPACE` so it can't
    /// collide with a real `[users] name = "..."` registration that happens
    /// to be called "root", "admin", "system", etc.
    #[test]
    fn root_api_key_user_id_does_not_collide_with_any_named_user() {
        use librefang_types::agent::UserId;
        // Any name an operator might plausibly use for the master account.
        for candidate in ["root", "admin", "owner", "system", "operator", "user"] {
            let from_name = UserId::from_name(candidate);
            assert_ne!(
                from_name.0, ROOT_API_KEY_USER_ID,
                "synthetic root id collides with UserId::from_name({candidate:?}) — \
                 operator with a {candidate} user would silently inherit master ACL"
            );
        }
    }

    /// #6631: an Admin who obtains credentials must not be able to install a plugin and then run its code as the daemon user.
    /// Every plugin route that puts plugin-controlled code on an execution path is Owner-only.
    #[test]
    fn admin_cannot_reach_plugin_routes_that_execute_plugin_code() {
        let post = axum::http::Method::POST;
        for path in [
            "/api/plugins/install",
            "/api/plugins/install-with-deps",
            "/api/plugins/prewarm",
            // Dispatches on a body field and accepts `enable` / `sign`, so leaving it open reaches those gates in bulk.
            "/api/plugins/batch",
            "/api/plugins/evil/install-deps",
            "/api/plugins/evil/test-hook",
            // `run_hook_json` in a loop — `test-hook` with a multiplier.
            "/api/plugins/evil/benchmark",
            "/api/plugins/evil/upgrade",
            "/api/plugins/evil/enable",
            "/api/plugins/evil/reload",
            "/api/plugins/evil/prewarm",
            "/api/plugins/evil/sign",
        ] {
            assert!(
                !user_role_allows_request(UserRole::Admin, &post, path),
                "Admin must NOT be allowed to POST {path} — it can execute \
                 plugin-controlled code under the daemon UID"
            );
            assert!(
                user_role_allows_request(UserRole::Owner, &post, path),
                "Owner must still be allowed to POST {path}"
            );
            for role in [UserRole::Viewer, UserRole::User] {
                assert!(
                    !user_role_allows_request(role, &post, path),
                    "{role} must NOT be allowed to POST {path}"
                );
            }
        }
    }

    /// The complement, and the part that is easy to get wrong: gating too much is also a security regression.
    /// An Admin must keep the ability to shut a malicious plugin off during an incident, and reads must stay readable.
    #[test]
    fn admin_retains_plugin_routes_that_remove_or_only_read_code() {
        let post = axum::http::Method::POST;
        for path in [
            // These REMOVE code from the execution path.
            // Owner-gating them would leave an Admin unable to respond to a compromise.
            "/api/plugins/uninstall",
            "/api/plugins/evil/disable",
            // Writes a template; executes nothing.
            "/api/plugins/scaffold",
        ] {
            assert!(
                user_role_allows_request(UserRole::Admin, &post, path),
                "Admin must retain POST {path} — gating it makes incident \
                 response harder without preventing code execution"
            );
        }

        let get = axum::http::Method::GET;
        for path in [
            "/api/plugins",
            "/api/plugins/registries",
            "/api/plugins/doctor",
            "/api/plugins/evil",
            "/api/plugins/evil/status",
            "/api/plugins/evil/lint",
            "/api/plugins/evil/env",
            "/api/context-engine/metrics",
        ] {
            assert!(
                user_role_allows_request(UserRole::Admin, &get, path),
                "Admin must retain GET {path}"
            );
        }
    }

    /// Extract every mutating route registered in `routes::plugins::router()`.
    ///
    /// Shared by the classification guard and by `plugin_route_extraction_is_line_ending_agnostic`, which is what makes the CRLF case provable without a Windows runner.
    ///
    /// Line endings are normalised first.
    /// `include_str!` yields the file's bytes verbatim and a Windows checkout stores CRLF, so the `"\n}\n"` body terminator matched nothing there — the guard panicked on `.expect` and the Windows shard was the only lane that went red.
    ///
    /// Returns an empty vec rather than panicking when the shape is unrecognisable: the caller asserts on the count, which produces a message naming what it did parse instead of an opaque `expect`.
    fn extract_mutating_plugin_routes(src: &str) -> Vec<(axum::http::Method, String)> {
        let src = src.replace("\r\n", "\n");
        let Some(body_start) = src.find("pub fn router()") else {
            return Vec::new();
        };
        let body = &src[body_start..];
        let Some(body_end) = body.find("\n}\n") else {
            return Vec::new();
        };
        let body = &body[..body_end];

        let mut out = Vec::new();
        // Each `.route(` chunk holds one path literal and its method calls.
        // Formatting splits these across lines, so operate per chunk rather than per line.
        for chunk in body.split(".route(").skip(1) {
            let Some(path_start) = chunk.find('"') else {
                continue;
            };
            let after = &chunk[path_start + 1..];
            let Some(path_len) = after.find('"') else {
                continue;
            };
            let route_path = &after[..path_len];
            if !route_path.starts_with("/plugins") {
                continue; // context-engine reads live in the same router
            }
            // Only the method calls for THIS route: `split` already stopped at the next `.route(` boundary.
            let methods_src = &after[path_len..];

            for (name, method) in [
                ("post", axum::http::Method::POST),
                ("put", axum::http::Method::PUT),
                ("patch", axum::http::Method::PATCH),
                ("delete", axum::http::Method::DELETE),
            ] {
                // `axum::routing::post(h)` for the first method on a route, `.delete(h)` for one chained onto it.
                if methods_src.contains(&format!("routing::{name}("))
                    || methods_src.contains(&format!(".{name}("))
                {
                    out.push((method, route_path.to_string()));
                }
            }
        }
        out
    }

    /// Regression for the Windows-only failure of the guard below.
    ///
    /// The bug was invisible on Linux and macOS because a checkout there stores LF, so a platform-independent test has to supply the CRLF form itself rather than rely on the host's line endings (refs #5716).
    #[test]
    fn plugin_route_extraction_is_line_ending_agnostic() {
        const PLUGINS_SRC: &str = include_str!("routes/plugins.rs");

        let lf_src = PLUGINS_SRC.replace("\r\n", "\n");
        let crlf_src = lf_src.replace('\n', "\r\n");
        assert!(
            crlf_src.contains("\r\n"),
            "the CRLF fixture must actually contain CRLF, or this proves nothing"
        );

        let from_lf = extract_mutating_plugin_routes(&lf_src);
        let from_crlf = extract_mutating_plugin_routes(&crlf_src);

        assert!(
            !from_lf.is_empty(),
            "extraction found no routes even with LF input — the parser is \
             broken independently of line endings"
        );
        assert_eq!(
            from_lf, from_crlf,
            "extraction must not depend on line endings: a Windows checkout \
             stores CRLF and previously yielded nothing here, which panicked \
             the classification guard on that shard only"
        );
    }

    /// Fail-closed completeness guard for the plugin authorization surface.
    ///
    /// The original #6631 fix enumerated the Owner-only set by reading `routes::plugins::router()` by hand, and missed `install-with-deps`, `prewarm` (both forms), `batch`, and `benchmark` — each a path to a capability that was already gated elsewhere.
    /// Adding those individually fixes the instances; it does nothing about the next route someone adds.
    ///
    /// So this reflects the actual route table out of the source and requires every mutating `/plugins/...` route to appear in exactly one of two explicit lists.
    /// A new route is a test failure until someone classifies it, which is the opposite of the silent-admission default that caused the misses.
    ///
    /// Two properties beyond "is each route classified":
    ///
    /// * Classification is decided by calling `user_role_allows_request` — the real gate — rather than `plugin_route_executes_plugin_code` directly, so the method dimension is exercised too.
    ///   The predicate is consulted only for POST, and asking it about a path in isolation would report a `PUT /plugins/{name}/enable` as gated when the gate would in fact wave an Admin straight through.
    /// * Every `ADMIN_ALLOWED` entry must be observed in the router.
    ///   A stale entry for a route that no longer exists (or never did with that method) is fail-open: it silently pre-classifies whatever is registered there next as Admin-safe, with no review.
    ///   Two such entries — a POST `lint` and a POST `health`, both GET-only in the router — were exactly that, and this assertion is what removed them.
    #[test]
    fn every_mutating_plugin_route_is_explicitly_classified() {
        const PLUGINS_SRC: &str = include_str!("routes/plugins.rs");

        // Routes that must stay reachable by Admin, each with its reason, keyed `METHOD path` so a route's classification cannot silently carry over to a different method registered on the same path.
        // Anything here is a deliberate decision, not an oversight.
        const ADMIN_ALLOWED: &[&str] = &[
            // Removes code from the execution path — Owner-gating it would block incident response.
            "POST /plugins/uninstall",
            "POST /plugins/{name}/disable",
            // Writes a template into the plugins dir; executes nothing.
            "POST /plugins/scaffold",
            // Clears the plugin's persisted key-value state.
            // Destroys data, runs no plugin code, and cannot make an unloadable hook loadable.
            "DELETE /plugins/{name}/state",
        ];

        // `include_str!` yields the file's bytes verbatim, and a Windows checkout stores CRLF, so `find("\n}\n")` matched nothing there and this test panicked on `.expect` — green on Linux and macOS, red on the Windows shard only.
        // Normalise once so every literal below is line-ending agnostic; `extract_mutating_plugin_routes` is the shared implementation so the CRLF case is provable without a Windows runner (see `plugin_route_extraction_is_line_ending_agnostic`).
        let routes = extract_mutating_plugin_routes(PLUGINS_SRC);

        let mut unclassified = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for (method, route_path) in routes {
            let key = format!("{} {route_path}", method.as_str());
            seen.push(key.clone());

            let full = format!("/api{route_path}");
            let admin_can = user_role_allows_request(UserRole::Admin, &method, &full);
            let listed = ADMIN_ALLOWED.contains(&key.as_str());
            if admin_can != listed {
                unclassified.push(format!(
                    "{key} (Admin reaches it: {admin_can}, in ADMIN_ALLOWED: {listed})"
                ));
            }
        }

        assert!(
            seen.len() >= 14,
            "parsed only {} mutating plugin routes — the extraction probably \
             broke rather than the router shrinking that much. Parsed: {seen:?}",
            seen.len()
        );
        assert!(
            unclassified.is_empty(),
            "these mutating /plugins routes are not classified exactly once. \
             Each must either execute plugin-controlled code (gate it in \
             `plugin_route_executes_plugin_code`) or be safe for Admin (add it \
             to ADMIN_ALLOWED here, with the reason):\n  {}",
            unclassified.join("\n  ")
        );
        let stale: Vec<&&str> = ADMIN_ALLOWED
            .iter()
            .filter(|entry| !seen.iter().any(|s| s == **entry))
            .collect();
        assert!(
            stale.is_empty(),
            "these ADMIN_ALLOWED entries match no route in \
             `routes::plugins::router()`. A stale entry pre-classifies a route \
             that does not exist yet as Admin-safe, so remove it:\n  {stale:?}"
        );
    }

    /// The Owner-only predicate keys on the action segment, so a plugin whose *name* happens to look like an action must not be mis-gated, and a deeper path must not slip through.
    #[test]
    fn plugin_owner_gate_matches_the_action_segment_not_the_name() {
        let post = axum::http::Method::POST;
        // `{name}` = "install-deps", action = "disable" → not Owner-only.
        assert!(
            user_role_allows_request(UserRole::Admin, &post, "/api/plugins/install-deps/disable"),
            "the gate must read the action segment, not any segment"
        );
        // Single-segment tail: `strip_prefix` leaves "enable" with no `/`, so there is no action segment and the predicate must fall through rather than treating the name as an action.
        // No POST route is registered at this shape today (`/plugins/{name}` is GET-only), so this is a predicate boundary rather than a reachable request — the point is that a future `POST /api/plugins/{name}` cannot be silently Owner-gated by a name that collides with an action.
        assert!(
            user_role_allows_request(UserRole::Admin, &post, "/api/plugins/enable"),
            "a single trailing segment carries no action, so the gate must not fire"
        );
    }

    #[test]
    fn test_user_role_admin_cannot_modify_config() {
        // Admin must be blocked from kernel-wide config mutations.
        let post = axum::http::Method::POST;
        for path in [
            "/api/config",
            "/api/config/set",
            "/api/config/reload",
            "/api/auth/change-password",
            "/api/shutdown",
        ] {
            assert!(
                !user_role_allows_request(UserRole::Admin, &post, path),
                "Admin must NOT be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_owner_still_allowed_on_config_writes() {
        let post = axum::http::Method::POST;
        for path in [
            "/api/config",
            "/api/config/set",
            "/api/config/reload",
            "/api/auth/change-password",
            "/api/shutdown",
        ] {
            assert!(
                user_role_allows_request(UserRole::Owner, &post, path),
                "Owner must be allowed to POST {path}"
            );
        }
    }

    // Finding #3: adding / updating / deleting an MCP server persists a stdio
    // transport whose `command` + `args` are spawned under the daemon UID.
    // That is process spawn — an Owner action — so an Admin ("config write")
    // must be blocked, matching the install-deps boundary.
    #[test]
    fn test_mcp_servers_mutations_are_owner_only() {
        // Only the config-mutation verbs are Owner-only: POST add on the
        // collection, PUT update / DELETE remove on a `{name}` target.
        let owner_only = [
            (axum::http::Method::POST, "/api/mcp/servers"),
            (axum::http::Method::PUT, "/api/mcp/servers/my-server"),
            (axum::http::Method::DELETE, "/api/mcp/servers/my-server"),
        ];
        for (method, path) in owner_only {
            for role in [UserRole::Viewer, UserRole::User, UserRole::Admin] {
                assert!(
                    !user_role_allows_request(role, &method, path),
                    "{role:?} must NOT be allowed to {method} {path}"
                );
            }
            assert!(
                user_role_allows_request(UserRole::Owner, &method, path),
                "Owner must be allowed to {method} {path}"
            );
        }
        // Reads (list / detail) stay at the generic Admin-or-above gate — the
        // GET short-circuit keeps them reachable for every role, so the gate
        // does not over-block the dashboard MCP page.
        let get = axum::http::Method::GET;
        for path in ["/api/mcp/servers", "/api/mcp/servers/my-server"] {
            for role in [
                UserRole::Viewer,
                UserRole::User,
                UserRole::Admin,
                UserRole::Owner,
            ] {
                assert!(
                    user_role_allows_request(role, &get, path),
                    "{role:?} must be allowed to GET {path}"
                );
            }
        }
        // Sub-resources that do not introduce a new spawn command keep their
        // existing Admin gate — an Admin may still reconnect / manage OAuth,
        // and is NOT forced up to Owner by the mutation gate.
        for (method, path) in [
            (
                axum::http::Method::POST,
                "/api/mcp/servers/my-server/reconnect",
            ),
            (
                axum::http::Method::DELETE,
                "/api/mcp/servers/my-server/auth/revoke",
            ),
        ] {
            assert!(
                user_role_allows_request(UserRole::Admin, &method, path),
                "Admin must still be allowed to {method} {path} (not a spawn-command mutation)"
            );
        }
    }

    // Finding #4: `/api/terminal/ws` and the tmux window endpoints are GET
    // routes that spawn / manage an interactive PTY under the daemon UID.
    // The blanket "GET is read-only" rule would wave a Viewer straight into a
    // shell, so they must require Admin+.
    #[test]
    fn test_terminal_privileged_gets_require_admin() {
        let get = axum::http::Method::GET;
        for path in [
            "/api/terminal/ws",
            "/api/terminal/windows",
            "/api/terminal/windows/main",
        ] {
            assert_eq!(min_role_for_privileged_get(path), Some(UserRole::Admin));
            for role in [UserRole::Viewer, UserRole::User] {
                assert!(
                    !user_role_allows_request(role, &get, path),
                    "{role:?} must NOT be allowed to GET {path} (interactive shell / PTY)"
                );
            }
            for role in [UserRole::Admin, UserRole::Owner] {
                assert!(
                    user_role_allows_request(role, &get, path),
                    "{role:?} must be allowed to GET {path}"
                );
            }
        }
        // The health probe is an ordinary read, not privileged.
        assert_eq!(min_role_for_privileged_get("/api/terminal/health"), None);
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/terminal/health"
        ));
    }

    // Finding #11: `GET /api/agents/{id}/ws` drives a full agent turn (LLM
    // calls, tool execution, budget spend) on inbound messages — the same
    // capability as REST `POST /api/agents/{id}/message`, which requires
    // User+. A Viewer is read-only and must be rejected on the WS too, closing
    // the RBAC inconsistency between the REST and WebSocket paths.
    #[test]
    fn test_agent_ws_requires_user_role() {
        let get = axum::http::Method::GET;
        let path = "/api/agents/abc123/ws";
        assert_eq!(min_role_for_privileged_get(path), Some(UserRole::User));
        // Viewer is blocked on the WS just as it is on POST /message.
        assert!(
            !user_role_allows_request(UserRole::Viewer, &get, path),
            "Viewer must NOT be allowed to GET the agent WS (drives LLM turns)"
        );
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &axum::http::Method::POST,
            "/api/agents/abc123/message"
        ));
        // User+ can drive turns over both the WS and REST.
        for role in [UserRole::User, UserRole::Admin, UserRole::Owner] {
            assert!(
                user_role_allows_request(role, &get, path),
                "{role:?} must be allowed to GET the agent WS"
            );
        }
        // A plain agent GET (not a WS upgrade) is an ordinary read.
        assert_eq!(min_role_for_privileged_get("/api/agents/abc123"), None);
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/agents/abc123"
        ));
    }

    /// `/api/config/export` returns the raw config.toml (inline secrets) and is
    /// a GET, so it must be Owner-gated — otherwise any authenticated
    /// Viewer / User / Admin could read the master api_key and escalate.
    #[test]
    fn test_config_export_is_owner_only() {
        let get = axum::http::Method::GET;
        let path = "/api/config/export";
        assert_eq!(min_role_for_privileged_get(path), Some(UserRole::Owner));
        for role in [UserRole::Viewer, UserRole::User, UserRole::Admin] {
            assert!(
                !user_role_allows_request(role, &get, path),
                "{role:?} must NOT be able to GET the raw config export (secrets)"
            );
        }
        assert!(
            user_role_allows_request(UserRole::Owner, &get, path),
            "Owner may export the raw config"
        );
        // The redacted `GET /api/config` stays readable by any role.
        assert_eq!(min_role_for_privileged_get("/api/config"), None);
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/config"
        ));
    }

    // Finding #20: unmatched requests must all collapse to a single bounded
    // Prometheus `path` label instead of leaking the concrete URI (which would
    // let unique-path spam mint unbounded, never-evicted metric series).
    #[test]
    fn test_unmatched_metric_path_collapses_to_constant() {
        // Matched routes keep their template.
        assert_eq!(
            metric_path_label(Some("/api/agents/{id}")),
            "/api/agents/{id}"
        );
        // Every unmatched request maps to the same constant — the helper has
        // no access to the raw URI, so distinct 404 paths cannot diverge.
        assert_eq!(metric_path_label(None), UNMATCHED_METRIC_PATH);
        assert_eq!(metric_path_label(None), metric_path_label(None));
        assert_ne!(UNMATCHED_METRIC_PATH, "/nope-0000001");
    }

    // #3621: TOTP enrollment must be Owner-only. Without this gate, any
    // bearer token (including a Viewer or User role) could overwrite the
    // unconfirmed `totp_secret` and hijack enrollment, or wipe a confirmed
    // enrollment via `revoke` and silently disable 2FA on login.
    #[test]
    fn test_totp_enrollment_is_owner_only() {
        let post = axum::http::Method::POST;
        for role in [UserRole::Viewer, UserRole::User, UserRole::Admin] {
            for path in [
                "/api/approvals/totp/setup",
                "/api/approvals/totp/confirm",
                "/api/approvals/totp/revoke",
            ] {
                assert!(
                    !user_role_allows_request(role, &post, path),
                    "{role:?} must NOT be allowed to POST {path}"
                );
            }
        }
        // Owner still has access.
        for path in [
            "/api/approvals/totp/setup",
            "/api/approvals/totp/confirm",
            "/api/approvals/totp/revoke",
        ] {
            assert!(
                user_role_allows_request(UserRole::Owner, &post, path),
                "Owner must be allowed to POST {path}"
            );
        }

        // Regression for over-gating: GET /api/approvals/totp/status is a
        // read-only enrollment-status probe and must remain reachable for
        // every authenticated role, including non-Owner ones.
        let get = axum::http::Method::GET;
        for role in [
            UserRole::Viewer,
            UserRole::User,
            UserRole::Admin,
            UserRole::Owner,
        ] {
            assert!(
                user_role_allows_request(role, &get, "/api/approvals/totp/status"),
                "{role:?} must be allowed to GET /api/approvals/totp/status"
            );
        }
    }

    // Install-deps spawns a package-manager process under the daemon UID
    // from argv that Admin can author in HAND.toml. Even with the
    // skill::install_hand_deps allowlist + flag denylist, this is the
    // wrong privilege boundary for an Admin role — restrict to Owner.
    // The matching `check-deps` sibling is read-only and stays at Admin.
    #[test]
    fn test_install_hand_deps_is_owner_only() {
        let post = axum::http::Method::POST;
        let install = "/api/hands/some-hand/install-deps";
        for role in [UserRole::Viewer, UserRole::User, UserRole::Admin] {
            assert!(
                !user_role_allows_request(role, &post, install),
                "{role:?} must NOT be allowed to POST {install}"
            );
        }
        assert!(
            user_role_allows_request(UserRole::Owner, &post, install),
            "Owner must be allowed to POST {install}"
        );

        // Sibling readiness probe stays Admin-accessible.
        let check = "/api/hands/some-hand/check-deps";
        assert!(
            user_role_allows_request(UserRole::Admin, &post, check),
            "Admin must still be allowed to POST {check} (read-only sibling)"
        );

        // Suffix-only matches must not over-restrict: the hands rule requires its own `/api/hands/` prefix, so a `/install-deps` elsewhere is not caught BY IT.
        // Isolating that needs a path under neither prefix.
        //
        // This assertion used `/api/plugins/foo/install-deps` as the example and asserted Admin could reach it.
        // That was only ever meant to prove the hands rule was not over-broad, but it pinned the plugin route as Admin-reachable — which is exactly the privilege boundary #6631 reported: an Admin could install a plugin and then run its package lifecycle scripts as the daemon user.
        // Plugin install-deps is now Owner-only through its own rule, so the example moved.
        let other = "/api/skills/some-skill/install-deps";
        assert!(
            user_role_allows_request(UserRole::Admin, &post, other),
            "the hands rule must require its own prefix rather than matching \
             any /install-deps suffix ({other})"
        );
        // And the plugin sibling is Owner-only, deliberately — asserted here as well as in the #6631 tests so a future edit to this test cannot quietly restore the old expectation.
        let plugin_deps = "/api/plugins/foo/install-deps";
        assert!(
            !user_role_allows_request(UserRole::Admin, &post, plugin_deps),
            "Admin must NOT reach {plugin_deps} (#6631)"
        );
    }

    #[test]
    fn test_user_role_admin_can_still_spawn_agents_and_install_skills() {
        let post = axum::http::Method::POST;
        for path in ["/api/agents", "/api/skills/install"] {
            assert!(
                user_role_allows_request(UserRole::Admin, &post, path),
                "Admin must still be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_user_still_limited_to_message_endpoints() {
        let post = axum::http::Method::POST;
        assert!(user_role_allows_request(
            UserRole::User,
            &post,
            "/api/agents/123/message"
        ));
        // Users still can't touch spawn, skill install, or config.
        for path in ["/api/agents", "/api/skills/install", "/api/config/set"] {
            assert!(
                !user_role_allows_request(UserRole::User, &post, path),
                "User must NOT be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_approval_suffixes_anchored_to_approvals_prefix() {
        // The `/approve`, `/reject`, `/modify`, … suffixes are meant only for
        // tool-call approval actions under `/api/approvals/`. Left unanchored
        // they also matched `/api/skills/pending/{id}/approve` and
        // `/api/a2a/agents/{id}/approve`, neither of which has an in-handler
        // role check — a User bearer could approve pending skills and trust
        // A2A agents (privilege escalation).
        let post = axum::http::Method::POST;
        for path in [
            "/api/skills/pending/x/approve",
            "/api/skills/pending/x/reject",
            "/api/a2a/agents/x/approve",
            "/api/a2a/agents/x/reject",
        ] {
            assert!(
                !user_role_allows_request(UserRole::User, &post, path),
                "User must NOT be allowed to POST {path}"
            );
        }
        // Genuine tool-call approval actions stay allowed for User.
        for path in [
            "/api/approvals/x/approve",
            "/api/approvals/batch",
            "/api/approvals/session/sess-1/approve_all",
            "/api/approvals/x/modify",
        ] {
            assert!(
                user_role_allows_request(UserRole::User, &post, path),
                "User must be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_admin_cannot_mutate_users_endpoints() {
        // RBAC M6: every mutating call under /api/users* maps to
        // Action::ManageUsers, which requires Owner. Without this gate an
        // Admin per-user API key could promote itself to Owner via
        // POST /api/users.
        for method in [
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ] {
            for path in ["/api/users", "/api/users/alice", "/api/users/import"] {
                assert!(
                    !user_role_allows_request(UserRole::Admin, &method, path),
                    "Admin must NOT be allowed to {method} {path}"
                );
                assert!(
                    user_role_allows_request(UserRole::Owner, &method, path),
                    "Owner must be allowed to {method} {path}"
                );
            }
        }
    }

    #[test]
    fn test_user_role_admin_cannot_mutate_groups_endpoints() {
        // #7745: a group's `roles` list confers role strings on its members, so
        // an Admin that could write groups could grant itself whatever role it
        // wanted and self-promote — the same escalation the `/api/users*` gate
        // above closes, one indirection further out.
        for method in [
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ] {
            for path in [
                "/api/groups",
                "/api/groups/oncall",
                "/api/groups/oncall/members/alice",
            ] {
                assert!(
                    !user_role_allows_request(UserRole::Admin, &method, path),
                    "Admin must NOT be allowed to {method} {path}"
                );
                assert!(
                    user_role_allows_request(UserRole::Owner, &method, path),
                    "Owner must be allowed to {method} {path}"
                );
            }
        }
    }

    #[test]
    fn test_group_reads_stay_at_the_generic_admin_gate() {
        // The dashboard's group list and the `/api/users/{name}/groups` reverse
        // lookup are reads, and locking them to Owner would make the surface
        // unusable for the Admin who is expected to operate it.
        let get = axum::http::Method::GET;
        for path in [
            "/api/groups",
            "/api/groups/oncall",
            "/api/users/alice/groups",
        ] {
            assert!(user_role_allows_request(UserRole::Admin, &get, path));
            assert!(user_role_allows_request(UserRole::Owner, &get, path));
        }
    }

    #[test]
    fn test_user_role_viewer_can_still_list_users_for_simulator() {
        // GET on /api/users* stays at the generic Admin-or-above gate (the
        // permission simulator needs the list). Viewer/User remain GET-only
        // by the existing user_role_allows_request rules.
        let get = axum::http::Method::GET;
        assert!(user_role_allows_request(
            UserRole::Admin,
            &get,
            "/api/users"
        ));
        assert!(user_role_allows_request(
            UserRole::Owner,
            &get,
            "/api/users"
        ));
        // GET is universally allowed by the role-allows logic, so even
        // Viewer can read — middleware-level filtering of PII is a
        // separate concern (UserView already redacts api_key_hash).
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/users"
        ));
    }

    #[test]
    fn test_user_role_viewer_still_get_only() {
        let get = axum::http::Method::GET;
        let post = axum::http::Method::POST;
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/agents"
        ));
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &post,
            "/api/agents/123/message"
        ));
        // Session-scoped approval endpoints are also denied for Viewer.
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &post,
            "/api/approvals/session/sess-1/approve_all"
        ));
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &post,
            "/api/approvals/session/sess-1/reject_all"
        ));
    }

    #[tokio::test]
    async fn test_api_version_header_prefers_explicit_path_version() {
        let app = Router::new()
            .route("/api/v1/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("accept", "application/vnd.librefang.v99+json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_api_version_header_rejects_unknown_vendor_version_on_alias() {
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("accept", "application/vnd.librefang.v99+json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[tokio::test]
    async fn test_api_version_header_accepts_vendor_media_type_with_parameters() {
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("accept", "application/vnd.librefang.v1+json; charset=utf-8")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_api_version_header_ignores_non_json_vendor_media_type() {
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("accept", "application/vnd.librefang.v1+xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_api_version_header_is_added_to_unauthorized_responses() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/private", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_user_api_key_can_post_agent_messages() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route(
                "/api/agents/123/message",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents/123/message")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_user_api_key_cannot_spawn_agents() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route(
                "/api/agents",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_viewer_api_key_cannot_post_anything() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "ReadOnly".to_string(),
                role: UserRole::Viewer,
                api_key_hash: crate::password_hash::hash_password("viewer-key").unwrap(),
                user_id: UserId::from_name("ReadOnly"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route(
                "/api/agents/123/message",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents/123/message")
                    .header("authorization", "Bearer viewer-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_viewer_api_key_can_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "ReadOnly".to_string(),
                role: UserRole::Viewer,
                api_key_hash: crate::password_hash::hash_password("viewer-key").unwrap(),
                user_id: UserId::from_name("ReadOnly"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/budget", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/budget")
                    .header("authorization", "Bearer viewer-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_trailing_slash_does_not_bypass_acl() {
        // Verify that a User-role key trying to POST /api/agents/ (with
        // trailing slash) still gets FORBIDDEN, not allowed through because
        // the path normalization strips the slash before the ACL check.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route(
                "/api/agents",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .route(
                "/api/agents/",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents/")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // After normalization "/api/agents/" → "/api/agents", which User
        // role is not allowed to POST to → FORBIDDEN.
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Regression for #2305: GET / must stay public. Earlier path
    /// normalization stripped the trailing slash from "/" producing an
    /// empty string, so the `path == "/"` public-endpoint check missed
    /// and the dashboard HTML returned 401 instead of the SPA.
    #[tokio::test]
    async fn test_root_path_is_public_even_with_api_key_set() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("somekey".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/", get(|| async { "dashboard html" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET / must serve the dashboard HTML without auth so the SPA can render"
        );
    }

    #[tokio::test]
    async fn test_forbidden_response_has_json_content_type() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route(
                "/api/agents",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["content-type"], "application/json");
    }

    /// With an api_key configured and `require_auth_for_reads = true`,
    /// GET /api/agents must stop being public — otherwise a remote caller
    /// on a 0.0.0.0 listener can enumerate agents without a token.
    #[tokio::test]
    async fn test_require_auth_for_reads_blocks_unauthenticated_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "require_auth_for_reads=true must make dashboard read endpoints \
             require a bearer token"
        );
    }

    /// With `require_auth_for_reads = true` the correct bearer still goes
    /// through, so legitimate dashboard clients keep working.
    #[tokio::test]
    async fn test_require_auth_for_reads_allows_authenticated_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// `/api/health` must stay reachable without a token even when
    /// `require_auth_for_reads = true` so probes, load balancers, and
    /// orchestrators can keep working.
    #[tokio::test]
    async fn test_require_auth_for_reads_keeps_health_public() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Default (flag off) behaviour must be preserved bit-for-bit: an
    /// unauthenticated GET /api/agents still succeeds so existing
    /// dashboards keep rendering.
    #[tokio::test]
    async fn test_require_auth_for_reads_off_preserves_public_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// `/api/auto-dream/status` is a dashboard read — same shape as
    /// `/api/agents` etc.: GET returns the global toggle + per-agent
    /// state, drives the Settings page's Dream Mode card. Must not 401
    /// when no auth is configured (default install) so the SPA renders.
    /// POST endpoints under `/api/auto-dream/agents/*` (trigger / abort /
    /// enabled) stay write-protected — they are not added to the
    /// allowlist.
    #[tokio::test]
    async fn test_auto_dream_status_get_is_dashboard_read_public() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/auto-dream/status", get(|| async { "status" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auto-dream/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// `/api/health/detail`'s own doc comment says "requires auth" and its
    /// payload includes panic counts, agent counts, model IDs, and
    /// `config_warnings` from `KernelConfig::validate()`. Unlike the
    /// dashboard-read group, this endpoint requires auth **unconditionally**
    /// — even when `require_auth_for_reads` is off — because its handler
    /// doc contract said so all along and the middleware was just wrong.
    /// `/api/health` stays public either way for load balancers.
    #[tokio::test]
    async fn test_api_health_detail_always_requires_auth() {
        // Flag OFF: /api/health is still public, /api/health/detail still
        // requires auth. This is the contract fix — it used to be in the
        // always-public set.
        let auth_state_off = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app_off = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .route("/api/health/detail", get(|| async { "detail" }))
            .layer(axum::middleware::from_fn_with_state(auth_state_off, auth));

        let health = app_off
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            health.status(),
            StatusCode::OK,
            "/api/health must stay public regardless of the flag"
        );

        let detail = app_off
            .oneshot(
                Request::builder()
                    .uri("/api/health/detail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            detail.status(),
            StatusCode::UNAUTHORIZED,
            "/api/health/detail must require auth even when the flag is off — \
             its doc comment has always said so"
        );

        // Flag ON: contract unchanged.
        let auth_state_on = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app_on = Router::new()
            .route("/api/health/detail", get(|| async { "detail" }))
            .layer(axum::middleware::from_fn_with_state(auth_state_on, auth));

        let detail = app_on
            .oneshot(
                Request::builder()
                    .uri("/api/health/detail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::UNAUTHORIZED);
    }

    /// `/api/status` used to be in the always-public set, but its handler
    /// returns the full agents listing + home_dir + api_listen — exactly
    /// the enumeration surface the flag exists to close. It must be locked
    /// down when the flag is on.
    #[tokio::test]
    async fn test_require_auth_for_reads_blocks_api_status() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/status", get(|| async { "status" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "/api/status leaks the agent list; must require auth when the flag is on"
        );
    }

    /// The flag must gate on any configured auth method, not just `api_key`.
    /// An operator with only per-user API keys (and empty `api_key`) must
    /// still get dashboard reads locked down when they enable the flag —
    /// gating on `api_key_present` alone would silently no-op here.
    #[tokio::test]
    async fn test_require_auth_for_reads_engages_with_user_api_keys_only() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "alice".into(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("alice-key").unwrap(),
                user_id: UserId::from_name("alice"),
            }])),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        // Unauthenticated → must be rejected.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "flag must engage when auth is configured via user_api_keys alone"
        );

        // Valid per-user key → must succeed.
        let ok = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .header("authorization", "Bearer alice-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    /// Flag is set but no auth of any kind is configured → must not
    /// accidentally start returning 401 for unauthenticated reads. The
    /// startup warning in server.rs covers operator-visible feedback; the
    /// middleware preserves the open-development default.
    #[tokio::test]
    async fn test_require_auth_for_reads_is_noop_without_any_auth() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "flag must not block unauthenticated reads when no auth is configured — \
             the startup warning handles operator feedback"
        );
    }

    // ---- openfang #1034 port: empty-api_key fail-closed coverage --------
    //
    // Helper builders + 6 scenarios specified by the security port:
    //   (a) loopback + no key      → 200
    //   (b) LAN IP + no key        → 401
    //   (c) public IP + no key     → 401
    //   (d) allow_no_auth=1        → 200 from any origin
    //   (e) configured key         → still does normal Bearer validation
    //   (f) missing ConnectInfo    → 401 (fail-closed, never open)

    fn no_auth_state() -> AuthState {
        AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        }
    }

    fn with_key_state(key: &str) -> AuthState {
        AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(key.to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        }
    }

    fn protected_router(state: AuthState) -> Router {
        Router::new()
            .route("/api/agents/1", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state, auth))
    }

    fn req_with_addr(ip: &str) -> Request<Body> {
        let addr: std::net::SocketAddr = format!("{ip}:40000").parse().unwrap();
        let mut req = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        req
    }

    /// (a) Empty api_key + loopback origin → 200. Single-user dev UX kept.
    #[tokio::test]
    async fn empty_key_allows_loopback() {
        let app = protected_router(no_auth_state());
        let resp = app.oneshot(req_with_addr("127.0.0.1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// (b) Empty api_key + LAN origin → 401. Closes the #1034 hole where a
    /// 192.168.x caller could hit every non-public endpoint.
    #[tokio::test]
    async fn empty_key_blocks_lan_origin() {
        let app = protected_router(no_auth_state());
        let resp = app.oneshot(req_with_addr("192.168.1.50")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// (c) Empty api_key + public IP origin → 401.
    #[tokio::test]
    async fn empty_key_blocks_public_origin() {
        let app = protected_router(no_auth_state());
        let resp = app.oneshot(req_with_addr("203.0.113.5")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// (d) `allow_no_auth = true` (i.e. LIBREFANG_ALLOW_NO_AUTH=1 at boot)
    /// opens the door from any origin. Operators must opt in explicitly.
    #[tokio::test]
    async fn empty_key_with_allow_no_auth_opens_lan() {
        let mut s = no_auth_state();
        s.allow_no_auth = true;
        let app = protected_router(s);
        let resp = app.oneshot(req_with_addr("10.0.0.9")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// (e) With an api_key configured, missing token → 401, valid bearer → 200.
    /// Confirms the new branch only fires on the no-auth code path.
    #[tokio::test]
    async fn configured_key_still_validates_bearer() {
        let app = protected_router(with_key_state("secret"));
        let resp = app
            .clone()
            .oneshot(req_with_addr("203.0.113.5"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let addr: std::net::SocketAddr = "203.0.113.5:40000".parse().unwrap();
        let mut authed = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        authed
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        let ok = app.oneshot(authed).await.unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    /// (f) ConnectInfo extension is missing → fail closed. The middleware
    /// must never treat unknown origin as loopback. Defense in depth in case
    /// upstream wiring changes (e.g. a future router skips
    /// `into_make_service_with_connect_info`).
    #[tokio::test]
    async fn empty_key_blocks_when_connect_info_missing() {
        let app = protected_router(no_auth_state());
        // No ConnectInfo extension inserted.
        let req = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ---- Regression tests for bug #3558: loopback bypass removed -----------

    /// Regression #3558: when an api_key IS configured, a loopback request
    /// with NO token must be rejected. The old code unconditionally let any
    /// loopback caller through; the fix removes that bypass so loopback goes
    /// through the same token check as every other origin.
    #[tokio::test]
    async fn configured_key_loopback_no_token_is_rejected() {
        let app = protected_router(with_key_state("secret"));
        let resp = app.oneshot(req_with_addr("127.0.0.1")).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "loopback with a configured api_key but no token must be 401, not bypassed"
        );
    }

    /// Regression #3558: when an api_key IS configured, a loopback request
    /// WITH the correct token must still succeed (the fix must not break
    /// legitimate loopback callers that present credentials).
    #[tokio::test]
    async fn configured_key_loopback_valid_token_is_allowed() {
        let app = protected_router(with_key_state("secret"));
        let addr: std::net::SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let mut req = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "loopback with a valid bearer token must still be allowed through"
        );
    }

    // ---- Bug #3781: GET /a2a/tasks/{id} must require auth ---------------
    //
    // Before the fix, `path.starts_with("/a2a/")` in the always_public_get_only
    // block let any caller read full task transcripts (agent prompts + LLM
    // outputs) without a bearer token. Only `/a2a/agents` (capability discovery)
    // should remain public; task-level resources contain sensitive data.

    /// GET /a2a/agents (the capability listing) must stay public — external
    /// A2A peers call this to discover what skills a local agent exposes.
    #[tokio::test]
    async fn a2a_agents_listing_is_always_public() {
        let app = Router::new()
            .route("/a2a/agents", get(|| async { "agent list" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/a2a/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET /a2a/agents must be public so external A2A peers can discover local agents"
        );
    }

    /// GET /a2a/tasks/{id} must require auth (Bug #3781). Task transcripts
    /// contain full agent prompts and LLM outputs — sensitive operational data.
    #[tokio::test]
    async fn a2a_task_transcript_requires_auth() {
        let app = Router::new()
            .route("/a2a/tasks/{id}", get(|| async { "full task transcript" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        // Unauthenticated → must be rejected.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/a2a/tasks/some-uuid-1234")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "GET /a2a/tasks/{{id}} must require auth — it returns full task transcripts"
        );
    }

    /// Regression for #3473 (dup of #3781): GET /a2a/tasks/{id}/status must
    /// also require auth. The status endpoint exposes per-task progress
    /// signals usable for side-channel inference even before the full
    /// transcript is fetched, so it has to share the auth gate.
    #[tokio::test]
    async fn a2a_task_status_requires_auth() {
        let app = Router::new()
            .route("/a2a/tasks/{id}/status", get(|| async { "task status" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/a2a/tasks/some-uuid-1234/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "GET /a2a/tasks/{{id}}/status must require auth (#3473 dup of #3781)"
        );
    }

    /// GET /a2a/tasks/{id} must allow access with a valid bearer token.
    #[tokio::test]
    async fn a2a_task_transcript_accessible_with_valid_token() {
        let app = Router::new()
            .route("/a2a/tasks/{id}", get(|| async { "full task transcript" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        let addr: std::net::SocketAddr = "203.0.113.5:40000".parse().unwrap();
        let mut req = Request::builder()
            .uri("/a2a/tasks/some-uuid-1234")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));

        let response = app.oneshot(req).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "valid bearer token must allow access to /a2a/tasks/{{id}}"
        );
    }

    // ---- Bug #3680: GET /api/logs/stream must require auth even when
    // ---- require_auth_for_reads = false -------------------------------
    //
    // Before #3909 the SSE endpoint was unconditionally appended to
    // `dashboard_read_public` (`|| path == "/api/logs/stream"`) so any
    // operator who explicitly set `require_auth_for_reads = false` (the
    // documented escape hatch for an external auth proxy) lost auth on
    // the log stream. The stream emits real-time tracing fields that can
    // contain prompts, OAuth callback codes, MCP stderr, and bearer
    // prefixes — a continuous credential leak. The fix removed the
    // path from every public allowlist; this test locks that contract
    // so a future refactor cannot silently re-introduce it.

    /// GET /api/logs/stream must return 401 when `require_auth_for_reads`
    /// is OFF — the SSE log stream is sensitive enough that the
    /// "loosen reads" escape hatch must NOT apply to it.
    #[tokio::test]
    async fn logs_stream_requires_auth_even_when_reads_are_loosened() {
        // Reproduce the deployment posture that exposed the bug:
        // an api_key is configured, but the operator has opted out of
        // auth-gating dashboard reads (e.g. fronting with an external
        // auth proxy). /api/logs/stream MUST still require auth.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/logs/stream", get(|| async { "sse stream" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        // Simulate a remote (non-loopback) caller so the loopback
        // short-circuit cannot mask the bug.
        let addr: std::net::SocketAddr = "203.0.113.5:53000".parse().unwrap();
        let mut req = Request::builder()
            .uri("/api/logs/stream")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "GET /api/logs/stream must require auth — SSE leaks tracing \
             fields with prompts, OAuth codes, and bearer prefixes"
        );
    }

    /// Sanity check: /api/logs/stream with a valid bearer DOES go through.
    /// Without this counter-test the regression test above could pass by
    /// accident (e.g. if the route were globally blocked).
    #[tokio::test]
    async fn logs_stream_allows_authenticated_caller() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/logs/stream", get(|| async { "sse stream" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let addr: std::net::SocketAddr = "203.0.113.5:53000".parse().unwrap();
        let mut req = Request::builder()
            .uri("/api/logs/stream")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "valid bearer token must allow access to /api/logs/stream"
        );
    }

    /// Regression: #3367 — GET /api/approvals/session/{id} used to be
    /// publicly readable via the `/api/approvals/` prefix in
    /// `dashboard_read_prefix`. That endpoint returns pending approval
    /// details including shell commands, so it must require authentication
    /// even when `require_auth_for_reads` is off.
    ///
    /// Updated post-#3941 audit: every approvals read endpoint exposes
    /// the same `action_summary` (pending shell command), so the entire
    /// `/api/approvals/*` surface must be auth-gated, not just the
    /// `/session/` sub-tree.
    #[tokio::test]
    async fn approvals_reads_require_auth() {
        // Auth state: api_key configured, require_auth_for_reads OFF — this
        // is the scenario where the bug was exploitable.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/approvals", get(|| async { "list" }))
            .route(
                "/api/approvals/session/{id}",
                get(|| async { "pending approvals" }),
            )
            .route("/api/approvals/audit", get(|| async { "audit log" }))
            .route("/api/approvals/{id}", get(|| async { "approval detail" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        for path in &[
            "/api/approvals",
            "/api/approvals/session/sess-abc-123",
            "/api/approvals/audit",
            "/api/approvals/some-approval-id",
        ] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(*path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must be auth-gated (returns action_summary)"
            );
        }
    }

    /// Regression: #5139 — `GET /api/cron/jobs` and
    /// `GET /api/cron/jobs/{id}` used to be publicly readable via the
    /// `/api/cron/` prefix in `PUBLIC_ROUTES_DASHBOARD_READS`. Those
    /// endpoints serialise the FULL `CronJob`, including the user-authored
    /// prompt (`CronAction::AgentTurn.message` / `SystemEvent.text`) and
    /// per-job `session_mode`. Same exposure class as the #3367/#3941
    /// approvals carve-out, so the entire `/api/cron/*` read surface must
    /// require auth even when `require_auth_for_reads` is off.
    #[tokio::test]
    async fn cron_reads_require_auth() {
        // api_key configured, require_auth_for_reads OFF — the exploitable
        // default scenario the audit flagged.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/cron/jobs", get(|| async { "cron jobs + prompts" }))
            .route(
                "/api/cron/jobs/{id}",
                get(|| async { "cron job detail + prompt_template" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        for path in &["/api/cron/jobs", "/api/cron/jobs/job-abc-123"] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(*path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must be auth-gated (leaks user-authored cron prompts)"
            );
        }
    }

    /// `/api/cron/` must not be present in the dashboard-reads allowlist —
    /// pins the data-level invariant so a future re-add is caught even if
    /// the routing test above is refactored.
    #[test]
    fn cron_prefix_absent_from_dashboard_reads() {
        assert!(
            !PUBLIC_ROUTES_DASHBOARD_READS.iter().any(|r| matches!(
                r.match_kind,
                PublicMatch::Prefix if r.path == "/api/cron/"
            )),
            "/api/cron/ must stay out of PUBLIC_ROUTES_DASHBOARD_READS (#5139)"
        );
    }

    /// Audit: check-json-depth-unused. The layer must reject deeply
    /// nested JSON before the handler sees it, but only when
    /// `Content-Type: application/json` is set. Other media types
    /// (multipart, text/plain, raw bytes) must pass through.
    #[tokio::test]
    async fn enforce_json_body_depth_rejects_payload_above_max_depth() {
        // Build a body with depth > MAX_JSON_BODY_DEPTH. Each level
        // wraps the next in an array so depth = nesting count.
        let deep_depth = MAX_JSON_BODY_DEPTH + 5;
        let mut body = String::from("0");
        for _ in 0..deep_depth {
            body = format!("[{body}]");
        }

        let app: Router = Router::new()
            .route("/echo", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(enforce_json_body_depth));

        let req = Request::post("/echo")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "deeply nested JSON must be rejected at the middleware boundary"
        );
    }

    #[tokio::test]
    async fn enforce_json_body_depth_accepts_payload_at_or_below_max_depth() {
        // Build a body at exactly MAX_JSON_BODY_DEPTH levels.
        let mut body = String::from("0");
        for _ in 0..MAX_JSON_BODY_DEPTH {
            body = format!("[{body}]");
        }
        let app: Router = Router::new()
            .route("/echo", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(enforce_json_body_depth));
        let req = Request::post("/echo")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn enforce_json_body_depth_ignores_non_json_content_type() {
        // The middleware must NOT buffer non-JSON requests. A deeply-
        // bracketed `text/plain` body that would trigger a depth-
        // exceeded JSON error must pass through untouched and reach
        // the handler.
        let mut body = String::from("x");
        for _ in 0..(MAX_JSON_BODY_DEPTH + 10) {
            body = format!("[{body}]");
        }
        let app: Router = Router::new()
            .route("/echo", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(enforce_json_body_depth));
        let req = Request::post("/echo")
            .header("content-type", "text/plain")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "non-JSON content types must skip the depth guard entirely"
        );
    }

    #[tokio::test]
    async fn enforce_json_body_depth_passes_malformed_json_through_to_handler() {
        // The middleware should NOT reject a malformed JSON body —
        // the handler's own deserializer will return a more specific
        // 4xx with the exact column. This test pins that contract:
        // the depth guard never observes a value, so it forwards.
        let app: Router = Router::new()
            .route("/echo", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(enforce_json_body_depth));
        let req = Request::post("/echo")
            .header("content-type", "application/json")
            .body(Body::from("{not valid"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Handler returns 200; the malformed JSON never matters here
        // because the test handler is `async { "ok" }` — it doesn't
        // deserialize. The point of this test is that the *middleware*
        // doesn't short-circuit a 400 on malformed JSON itself.
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn request_logging_sees_matched_route_template_not_free_text_param() {
        // Regression for finding #14: the metric `path` label must be built
        // from the matched route TEMPLATE, not the concrete URI, so a
        // free-text route param never inflates Prometheus label cardinality.
        //
        // `request_logging` reads `MatchedPath` from the request extensions
        // before calling `next.run`; the inner handler reads the very same
        // extension. axum inserts `MatchedPath` during routing (before any
        // `Router::layer` middleware runs), so asserting the handler sees the
        // template pins the exact value `request_logging` forwards to
        // `record_http_request`. Before the fix the concrete path
        // (`/api/models/aliases/some-free-text-alias`) was used verbatim.
        use axum::extract::MatchedPath;

        async fn echo_matched_path(matched: MatchedPath) -> String {
            matched.as_str().to_string()
        }

        let app: Router = Router::new()
            .route("/api/models/aliases/{alias}", get(echo_matched_path))
            .layer(axum::middleware::from_fn(request_logging));

        let req = Request::get("/api/models/aliases/some-free-text-alias")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "/api/models/aliases/{alias}",
            "request_logging must observe the route template, not the free-text param"
        );
    }

    /// Regression for #4860: the inline login page must redirect to `/`
    /// (the SPA shell) when it was itself served at `/`, `/dashboard`, or
    /// `/dashboard/`. The router only registers `/` and
    /// `/dashboard/{*path}`, so redirecting back to `/dashboard` or
    /// `/dashboard/` after a successful sign-in lands on a 404.
    #[test]
    fn login_page_redirects_dashboard_root_to_spa_shell() {
        let html = super::LOGIN_PAGE_HTML;
        // Pin the full collapse condition so neither the bare `/dashboard`
        // case nor the trailing-slash case can be silently dropped — a
        // substring like `path === '/dashboard'` would also match
        // `path === '/dashboard/'` and let one half regress unnoticed.
        assert!(
            html.contains("path === '/dashboard' || path === '/dashboard/'"),
            "login page must collapse both /dashboard and /dashboard/ to the SPA shell at /"
        );
        assert!(
            !html.contains("target = '/dashboard/';"),
            "login page must not redirect to /dashboard/ — that path 404s (#4860)"
        );
    }

    /// Regression for #6477: the `@media (prefers-color-scheme: light)` block
    /// must be declared AFTER the base `.card` rule. CSS resolves equal-
    /// specificity conflicts by source order, so a light override placed
    /// before the dark base rule loses in light mode — the page background
    /// turns light while `.card`/`input` keep their dark base values, leaving
    /// dark heading/label text invisible on a dark card. Assert the ordering
    /// in the source so the block can never drift back above the base rules.
    #[test]
    fn login_page_light_theme_block_follows_base_card_rule() {
        let html = super::LOGIN_PAGE_HTML;
        let base_card = html
            .find(".card {")
            .expect("login page must define a base `.card` rule");
        let light_media = html
            .find("@media (prefers-color-scheme: light)")
            .expect("login page must define a light-theme media block");
        assert!(
            light_media > base_card,
            "the light-theme media block must come AFTER the base `.card` rule so \
             its overrides win the cascade in light mode (#6477); found media block \
             at byte {light_media}, base .card at byte {base_card}"
        );
        // The light block must actually re-style the surfaces that carry dark
        // base values, or light mode leaves unreadable text on a dark card.
        let light_block = &html[light_media..];
        for needed in [".card {", "input {", ".sub {", ".foot {"] {
            assert!(
                light_block.contains(needed),
                "light-theme media block must override `{needed}` (#6477)"
            );
        }
    }

    /// The `/dashboard` login page depends on its inline submit handler.
    /// The CSP must allow its exact hash without permitting arbitrary inline JavaScript.
    #[tokio::test]
    async fn dashboard_login_page_script_is_allowed_by_csp_hash() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};

        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            master_key: Default::default(),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: true,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/dashboard", get(|| async { "dashboard shell" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth))
            .layer(axum::middleware::from_fn(security_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let csp = response
            .headers()
            .get("content-security-policy")
            .and_then(|value| value.to_str().ok())
            .expect("login response must carry a CSP header")
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        let script = html
            .split_once("<script>")
            .and_then(|(_, rest)| rest.split_once("</script>"))
            .map(|(script, _)| script)
            .expect("login page must contain its submit handler");
        let digest = Sha256::digest(script.as_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(digest);
        let hash_source = format!("'sha256-{encoded}'");

        assert!(
            csp.split(';')
                .find(|directive| directive.trim_start().starts_with("script-src "))
                .is_some_and(|directive| directive
                    .split_ascii_whitespace()
                    .any(|source| source == hash_source.as_str())),
            "script-src must allow the exact inline login handler hash"
        );
        assert!(
            !csp.split(';')
                .find(|directive| directive.trim_start().starts_with("script-src "))
                .is_some_and(|directive| directive.contains("'unsafe-inline'")),
            "script-src must not allow arbitrary inline JavaScript"
        );
    }
}
