//! Cost-aware rate limiting using GCRA (Generic Cell Rate Algorithm).
//!
//! Each API operation has a token cost (e.g., health=1, spawn=50, message=30).
//! The GCRA algorithm allows 500 tokens per minute per IP address.
//!
//! Two bypass paths:
//!
//! - Path-based: non-API paths (dashboard SPA assets, locale JSON, favicon,
//!   logo, root) are exempt — a single dashboard page load fans out to
//!   dozens of static-asset requests and the default fallback cost drains
//!   the budget before the page finishes rendering. See
//!   [`is_rate_limit_exempt`].
//! - IP-based: direct loopback callers (127.0.0.0/8 and ::1, with no
//!   forwarding headers in the request) bypass the limiter, since they're
//!   local processes (dashboard SPA, librefang CLI, cron) calling their
//!   own daemon. The forwarding-header guard means a same-host reverse
//!   proxy that injects `X-Forwarded-For` / `X-Real-IP` does NOT trigger
//!   the bypass — proxied traffic still falls through to the limiter.
//!   See [`gcra_rate_limit`].
//!
//! A separate, stricter per-IP counter specifically for authentication
//! endpoints ([`auth_rate_limit_layer`]) limits login attempts to a
//! configurable number per 15-minute window. This provides brute-force
//! protection independent of the general token budget. See [`AuthLoginLimiter`].
//!
//! It also covers two of the endpoints that verify a TOTP or recovery code:
//! `/api/approvals/totp/confirm` is always counted, while
//! `/api/approvals/{id}/approve` is counted only while the approval policy
//! actually demands a code from the tool being approved — the per-request
//! condition is documented on [`auth_rate_limit_layer`].
//!
//! Two more verify codes and are deliberately *not* counted here:
//! `/api/approvals/totp/setup` (the `current_code` a re-enrollment sends) and
//! `/api/approvals/totp/revoke`. Each is an Owner-only write an operator
//! performs once, and each already fails closed on its own per-endpoint TOTP
//! lockout — five failed codes, then a five-minute refusal
//! (`ApprovalManager::check_and_record_totp_failure` under `SETUP_LOCKOUT_KEY`
//! / `REVOKE_LOCKOUT_KEY`) — so adding them to this counter would layer a
//! fifteen-minute brake on top of that for no gain in coverage.

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request, Response, StatusCode};
use axum::middleware::Next;
use dashmap::DashMap;
use governor::{clock::DefaultClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use std::borrow::Cow;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Paths exempt from rate limiting.
///
/// The dashboard SPA and its static support files are served from the same
/// Axum router as the API, so the rate-limit middleware sees every asset
/// request. Counting each one at `operation_cost`'s fallback of 5 tokens
/// exhausts the default 500-token/minute budget after roughly 20 assets —
/// well under what a cold SPA load fetches. These paths short-circuit the
/// limiter entirely; protocol, webhook, and `/api/*` paths continue to be
/// metered.
pub fn is_rate_limit_exempt(path: &str) -> bool {
    path == "/"
        || path == "/favicon.ico"
        || path == "/logo.png"
        || path.starts_with("/dashboard/")
        || path.starts_with("/locales/")
}

pub fn operation_cost(method: &str, path: &str) -> NonZeroU32 {
    match (method, path) {
        (_, "/api/health") => NonZeroU32::new(1).unwrap(),
        // Priced with `/api/health`, not at the 5-token fallback: a
        // Kubernetes pod runs startup + liveness + readiness probes
        // concurrently, and all three arrive from the node's kubelet — one
        // source IP, which is what the GCRA limiter keys on. At the fallback
        // cost a startup probe on a 1s period alone would drain the
        // 500-token/min budget and 429 the pod out of its own rollout.
        // Both spellings are listed because `operation_cost` matches the raw
        // path with no version normalisation, and a probe URL is written by
        // hand into a manifest — an operator who reaches for the versioned
        // form should not silently get the expensive tier.
        (_, "/api/ready") | (_, "/api/v1/ready") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/status") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/version") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/tools") => NonZeroU32::new(1).unwrap(),
        // High-frequency dashboard reads. The dashboard SPA polls these
        // every few seconds (TanStack Query refetchOnFocus + interval
        // refetch), and they're aggregating reads — not per-record
        // queries — so the work is constant-cost regardless of fleet
        // size. Pricing them at the fallback of 5 tokens drained the
        // 500-token/min budget in seconds and made the dashboard 429
        // out as soon as a couple of tabs were open. See #3416.
        ("GET", "/api/dashboard/snapshot") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/approvals/count") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/providers") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/media/providers") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/agents") => NonZeroU32::new(2).unwrap(),
        ("GET", "/api/skills") => NonZeroU32::new(2).unwrap(),
        ("GET", "/api/peers") => NonZeroU32::new(2).unwrap(),
        ("GET", "/api/config") => NonZeroU32::new(2).unwrap(),
        ("GET", "/api/usage") => NonZeroU32::new(3).unwrap(),
        ("GET", p) if p.starts_with("/api/audit") => NonZeroU32::new(5).unwrap(),
        ("GET", p) if p.starts_with("/api/marketplace") => NonZeroU32::new(10).unwrap(),
        ("POST", "/api/agents") => NonZeroU32::new(50).unwrap(),
        // Mobile pairing redemption: public (in `is_public` allowlist) and
        // mints a per-device bearer on success. Token entropy already
        // makes blind brute-force infeasible, but a 50-token charge caps
        // attempts at ~10/min per IP so a misbehaving / leaked client
        // can't hammer the endpoint either. The matching `/request`
        // endpoint stays on the default cost — it requires auth, so
        // abuse is bounded by the caller's existing role.
        ("POST", "/api/pairing/complete") => NonZeroU32::new(50).unwrap(),
        ("POST", p) if p.contains("/message") => NonZeroU32::new(30).unwrap(),
        ("POST", p) if p.contains("/run") => NonZeroU32::new(100).unwrap(),
        ("POST", "/api/skills/install") => NonZeroU32::new(50).unwrap(),
        ("POST", "/api/skills/uninstall") => NonZeroU32::new(10).unwrap(),
        ("POST", "/api/migrate") => NonZeroU32::new(100).unwrap(),
        // PATCH /api/agents/{id} accepts full-manifest replacement
        // (`manifest_toml`) — same heavyweight write the legacy
        // PUT /update endpoint used to handle (#3748). Match
        // `/api/agents/<id>` exactly (no trailing sub-path).
        ("PATCH", p)
            if p.starts_with("/api/agents/") && !p["/api/agents/".len()..].contains('/') =>
        {
            NonZeroU32::new(10).unwrap()
        }
        _ => NonZeroU32::new(5).unwrap(),
    }
}

/// Detect a forwarding header injected by an upstream reverse proxy.
///
/// Used by [`gcra_rate_limit`] to disqualify the loopback bypass: if a
/// proxy is in front, the loopback peer represents arbitrary public
/// callers, not a trusted local process. Returns `true` for any of
/// `X-Forwarded-For`, `X-Real-IP`, or `Forwarded` (RFC 7239).
fn has_forwarding_header(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key("forwarded")
}

fn rate_limit_response(error: &str, retry_after_secs: u64) -> Response<Body> {
    let mut response = Response::new(Body::from(serde_json::json!({"error": error}).to_string()));
    *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_str(&retry_after_secs.to_string())
            .expect("a decimal u64 is always a valid HTTP header value"),
    );
    response
}

pub type KeyedRateLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

/// Shared state for the GCRA rate limiting middleware layer.
#[derive(Clone)]
pub struct GcraState {
    pub limiter: Arc<KeyedRateLimiter>,
    pub retry_after_secs: u64,
    /// Compiled `trusted_proxies` allowlist. Empty (default) disables
    /// header-based client-IP resolution and the limiter falls back to
    /// the TCP peer for keying.
    pub trusted_proxies: Arc<crate::client_ip::TrustedProxies>,
    /// Operator opt-in for forwarding-header trust. Without this AND
    /// a non-empty `trusted_proxies`, header trust is off.
    pub trust_forwarded_for: bool,
}

/// Create a GCRA rate limiter with the given token budget per minute per IP.
pub fn create_rate_limiter(tokens_per_minute: u32) -> Arc<KeyedRateLimiter> {
    let quota = tokens_per_minute.max(1);
    Arc::new(RateLimiter::keyed(Quota::per_minute(
        NonZeroU32::new(quota).unwrap(),
    )))
}

/// GCRA rate limiting middleware.
///
/// Extracts the client IP from `ConnectInfo`, computes the cost for the
/// requested operation, and checks the GCRA limiter. Returns 429 if the
/// client has exhausted its token budget. Paths flagged by
/// [`is_rate_limit_exempt`] (static SPA assets, locale files, root, favicon,
/// logo) bypass the limiter entirely.
pub async fn gcra_rate_limit(
    axum::extract::State(state): axum::extract::State<GcraState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let path = request.uri().path().to_string();
    if is_rate_limit_exempt(&path) {
        return next.run(request).await;
    }

    // Resolve the real client IP. When the daemon sits behind a trusted
    // reverse proxy (config: `trusted_proxies` + `trust_forwarded_for`)
    // and the TCP peer matches the allowlist, this returns the IP from
    // the forwarding headers. Otherwise it returns the TCP peer — the
    // same conservative behavior as before, including the
    // `0.0.0.0`-on-missing-ConnectInfo fallback documented previously.
    // An untrusted peer can never use forwarding headers to forge a
    // unique source per request and bypass the limiter.
    let ip = crate::client_ip::resolve_from_request(
        &request,
        &state.trusted_proxies,
        state.trust_forwarded_for,
    );

    // Loopback (127.0.0.0/8 + ::1) bypasses the limiter. The dashboard
    // SPA, the librefang CLI, and any other process on the same host
    // talking to its own daemon all surface as loopback, and there's no
    // hostile-burst threat model from a peer that already has local
    // process privileges. Without this, a single dashboard tab refresh
    // (snapshot + approvals/count + providers + media/providers + …,
    // re-fetched on focus + interval) drained the 500-token/min budget
    // in seconds and 429'd the whole UI. See #3416.
    //
    // Reverse-proxy guard: if the request carries `X-Forwarded-For`,
    // `X-Real-IP`, or RFC 7239 `Forwarded`, the loopback peer is almost
    // certainly a same-host proxy (nginx / caddy / traefik) forwarding
    // traffic from arbitrary public clients. Bypassing in that case
    // would silently disable rate limiting for the whole internet. We
    // don't trust those headers to identify the *real* client (no
    // config-pinned trusted-proxy list yet), but their mere presence is
    // enough to disqualify the bypass — the limiter still runs against
    // the proxy's loopback IP, which makes proxied traffic share one
    // bucket. Less granular than per-real-IP metering, but strictly
    // safer than wide-open.
    if ip.is_loopback() && !has_forwarding_header(request.headers()) {
        return next.run(request).await;
    }

    let method = request.method().as_str().to_string();
    let cost = operation_cost(&method, &path);

    // `check_key_n` returns a nested `Result<Result<(), NotUntil>, InsufficientCapacity>`:
    //   * outer `Err(InsufficientCapacity)` — the cost exceeds the configured
    //     burst size; this request can never be served.
    //   * outer `Ok(Err(NotUntil))`         — the key is out of tokens right
    //     now; this is the **normal rate-limit trigger** we need to honour.
    //   * outer `Ok(Ok(()))`                — a token was consumed, pass
    //     through.
    //
    // The previous check — `state.limiter.check_key_n(&ip, cost).is_err()` —
    // only caught `InsufficientCapacity`, so `NotUntil` (the normal "you've
    // exhausted your quota" signal) was treated as OK and every request
    // slid straight through. A burst of 200 `/api/health` calls (cost=1,
    // quota=500/min) never returned 429 in practice, and heavier endpoints
    // (POST /api/agents at cost=50) were equally unthrottled until the
    // per-call cost itself grew larger than the burst size.
    let rate_limited = match state.limiter.check_key_n(&ip, cost) {
        Ok(Ok(())) => false,
        Ok(Err(_not_until)) => true,
        Err(_insufficient_capacity) => true,
    };
    if rate_limited {
        tracing::warn!(ip = %ip, cost = cost.get(), path = %path, "GCRA rate limit exceeded");
        return rate_limit_response("Rate limit exceeded", state.retry_after_secs);
    }

    next.run(request).await
}

// ── Per-IP auth rate limiter ──────────────────────────────────────────────────

/// Window size for the per-IP auth rate limiter.
pub const AUTH_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Retry-After value advertised to blocked callers (seconds).
pub const AUTH_RATE_LIMIT_RETRY_AFTER_SECS: u64 = 15 * 60;

/// Per-IP login attempt counter used by [`AuthLoginLimiter`].
#[derive(Debug, Clone)]
pub struct LoginAttempt {
    /// Number of attempts in the current window.
    pub count: u32,
    /// When the current window started.
    pub window_start: Instant,
}

/// Shared state for the per-IP auth rate limiter.
///
/// Stored as `Arc<AuthLoginLimiter>` in `AppState` so it can be accessed by
/// the `dashboard_login` handler and the auth-endpoint middleware layer. A
/// background task prunes stale entries (windows older than 30 minutes) to
/// prevent unbounded memory growth.
#[derive(Debug, Clone, Default)]
pub struct AuthLoginLimiter {
    /// Maps client IP → current-window attempt record.
    pub map: Arc<DashMap<IpAddr, LoginAttempt>>,
}

impl AuthLoginLimiter {
    /// Create a new, empty limiter.
    pub fn new() -> Self {
        Self {
            map: Arc::new(DashMap::new()),
        }
    }

    /// Record one attempt from `ip` and return whether the caller has exceeded
    /// `max_attempts` within the current 15-minute window.
    ///
    /// Returns `true` when the caller is over-limit (should be rejected with
    /// HTTP 429). Returns `false` when the attempt is within budget.
    ///
    /// When `max_attempts == 0` the limiter is disabled and every call returns
    /// `false`.
    pub fn check_and_record(&self, ip: IpAddr, max_attempts: u32) -> bool {
        if max_attempts == 0 {
            return false;
        }
        let now = Instant::now();
        let mut entry = self.map.entry(ip).or_insert(LoginAttempt {
            count: 0,
            window_start: now,
        });
        // Reset counter when the window has expired.
        if now.duration_since(entry.window_start) >= AUTH_RATE_LIMIT_WINDOW {
            entry.count = 0;
            entry.window_start = now;
        }
        entry.count += 1;
        entry.count > max_attempts
    }

    /// Remove entries whose window started more than 30 minutes ago. Called
    /// periodically by the background pruning task in `server.rs`.
    pub fn prune_stale(&self) {
        let cutoff = Duration::from_secs(30 * 60);
        let now = Instant::now();
        self.map
            .retain(|_, attempt| now.duration_since(attempt.window_start) < cutoff);
    }
}

/// Shared state for the auth-endpoint rate limiter middleware. Bundles
/// the limiter, the per-IP attempt cap, the trusted-proxy configuration
/// that decides whether forwarding headers can override the TCP peer for
/// keying, and the question of whether an approval request verifies a
/// code at all.
#[derive(Clone)]
pub struct AuthRateLimitState {
    pub limiter: Arc<AuthLoginLimiter>,
    pub max_attempts: u32,
    pub trusted_proxies: Arc<crate::client_ip::TrustedProxies>,
    pub trust_forwarded_for: bool,
    /// Whether approving the request with the given id would verify a TOTP or
    /// recovery code right now — i.e. whether the approval's tool is inside
    /// `ApprovalPolicy::tool_requires_totp` and the caller is outside the TOTP
    /// grace window.
    ///
    /// A predicate rather than a `bool` because the answer is neither a boot
    /// constant nor a policy-wide one: `POST /api/config/reload` swaps the
    /// whole approval policy through `ApprovalManager::update_policy`,
    /// `totp_tools` narrows which tools verify a code, and a successful
    /// verification opens a grace window that skips the next ones. A global
    /// answer kept metering approvals that verified nothing — the lockout this
    /// gates. It takes the approval id so each approve request is answered for
    /// itself.
    pub approvals_require_totp: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

/// The approval id in an `/api/approvals/{id}/approve` (or `/api/v1/...`)
/// path, if the path is one.
///
/// The segment is percent-decoded the way axum's `Path<String>` extractor
/// decodes it (`percent_decode_str` plus a strict UTF-8 check) before the
/// caller parses it. The middleware reads `request.uri().path()` — the raw
/// path, still percent-encoded — while the handler binds the already-decoded
/// segment. Without this step an id sent with an escape inside it (say a
/// `%2D` for one of a UUID's hyphens) fails `Uuid::parse_str` here but not in
/// `approve_request`, the gate answers `false`, and the request that verifies
/// a code rides the unmetered path. Decoding here keeps both ends parsing the
/// same string.
///
/// This is the id [`AuthRateLimitState::approvals_require_totp`] is asked
/// about: whether approving *that* request would verify a code.
fn approve_path_request_id(path: &str) -> Option<Cow<'_, str>> {
    let rest = path
        .strip_prefix("/api/approvals/")
        .or_else(|| path.strip_prefix("/api/v1/approvals/"))?;
    let id = rest.strip_suffix("/approve")?;
    // Decode before the shape checks: a raw `%2F` only becomes a segment
    // separator once decoded, and the handler's `{id}` carries the decoded
    // slash — asking the gate about a multi-segment id would be a different
    // request than the one `approve_request` resolves.
    let id = percent_encoding::percent_decode_str(id)
        .decode_utf8()
        .ok()?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// Axum middleware that enforces per-IP rate limiting on authentication
/// endpoints (`/api/auth/dashboard-login`, `/api/auth/login*`,
/// `/api/auth/introspect`, `/api/auth/refresh`, the OAuth callback, the
/// passkey ceremonies, `/api/approvals/totp/confirm`), and on
/// `/api/approvals/{id}/approve` while approving that request would have a
/// code to verify.
///
/// The approval endpoints are the ones that need a condition attached.
/// `approve_request` verifies a TOTP or recovery code only when the approval
/// policy says the tool being approved needs one
/// (`ApprovalPolicy::tool_requires_totp`, which `totp_tools` narrows) and the
/// caller is outside the grace window a previous successful verification
/// opened; with the shipped default `second_factor = none` no tool qualifies,
/// so an approval is an authenticated click that verifies nothing and there is
/// no credential on that path to brute-force. This middleware also runs
/// *before* the handler, so it cannot see the outcome: it was counting
/// successful approvals, and enough of them in one window exhausted the bucket
/// shared with `dashboard-login` and answered a working operator with 429 "Too
/// many login attempts" for the rest of the fifteen minutes. Metering is
/// therefore gated per request on
/// [`AuthRateLimitState::approvals_require_totp`], asked with the approval id
/// from the path — the same per-tool, grace-aware question the handler and
/// `resolve` answer. `totp/confirm` is not gated: it always
/// verifies a code when it is reachable, and it runs once per enrollment, so
/// the bucket costs a legitimate caller nothing.
///
/// `totp/setup` and `totp/revoke` are not in the set at all, though both
/// verify a code too — see the module docs for why the count keeps them on
/// their own lockout instead.
///
/// Loopback callers are exempted — the CLI and SPA connecting to their own
/// daemon must never be locked out. Non-loopback clients that have exceeded
/// `max_attempts` within the 15-minute window receive HTTP 429 with a
/// `Retry-After` header.
///
/// IP resolution: TCP peer by default. When the daemon is configured
/// with `trusted_proxies` + `trust_forwarded_for`, peers that match
/// the allowlist have their IP replaced with the value from the
/// forwarding headers (`CF-Connecting-IP` → `X-Real-IP` → `Forwarded`
/// → rightmost-untrusted `X-Forwarded-For`). Untrusted peers always
/// key on their own TCP source, so a forged `X-Forwarded-For` from
/// the open internet still collapses every spoof attempt onto one
/// bucket — the limiter's safety property is preserved.
pub async fn auth_rate_limit_layer(
    axum::extract::State(state): axum::extract::State<AuthRateLimitState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let limiter = state.limiter.clone();
    let max_attempts = state.max_attempts;
    let path = request.uri().path();
    // Asked lazily, and only for an approve path: the predicate takes the
    // kernel's approval-policy lock, so every other route in the allowlist
    // below must reach its answer without paying for it. The question is
    // per-request (tool + grace window) — see the middleware docs above.
    let approve_is_metered = approve_path_request_id(path)
        .is_some_and(|approval_id| (state.approvals_require_totp)(approval_id.as_ref()));

    // Endpoints that accept credentials, recovery codes, or TOTP codes —
    // any of these is a brute-force surface and must be rate-limited
    // alongside the password endpoints.  TOTP/recovery-code endpoints
    // were missing in #3950, leaving 6-digit-code brute force unbounded
    // for any session that already cleared the password gate.
    let is_auth_path = path == "/api/auth/dashboard-login"
        || path == "/api/v1/auth/dashboard-login"
        || path.starts_with("/api/auth/login")
        || path.starts_with("/api/v1/auth/login")
        || path == "/api/auth/introspect"
        || path == "/api/v1/auth/introspect"
        || path == "/api/auth/refresh"
        || path == "/api/v1/auth/refresh"
        // OAuth callback (audit: auth-callback-no-rate-limit).
        // /api/auth/callback MUST be public (the IdP redirect lands
        // here), but every successful HMAC verify eagerly consumes
        // the `oauth_nonce_used` slot BEFORE the actual code
        // exchange (`oauth.rs:744-758`). A captured `state` token
        // (referer leak, proxy log, browser history) can be
        // replayed from the open internet 50 ms before the
        // legitimate redirect arrives — the real user then sees
        // "OAuth callback already redeemed" with no remediation.
        // Free login DoS. Rate-limiting the callback gives a
        // per-IP brake on the replay window without breaking the
        // legitimate IdP redirect (which arrives once per login).
        || path == "/api/auth/callback"
        || path == "/api/v1/auth/callback"
        // #4020 added the approve paths for the reason above — they accept
        // 6-digit and recovery codes. They only *do* while the policy
        // requires one for the tool being approved and the caller is outside
        // the grace window: with `second_factor = none`/`login`, or a tool
        // outside a narrowed `totp_tools`, `tool_requires_totp` short-circuits
        // to false, nothing is verified, and metering the path bought no
        // brute-force protection while spending the caller's login budget on
        // approvals that had already succeeded. Gate on the same per-request
        // check the handler makes; keep the confirm path unconditional, since
        // it verifies a code whenever it is reachable and runs once per
        // enrollment.
        || approve_is_metered
        || path == "/api/approvals/totp/confirm"
        || path == "/api/v1/approvals/totp/confirm"
        // #5981: passkey authentication mints a session, so its two ceremony
        // endpoints are a login brute-force surface — meter them alongside
        // dashboard-login.
        || path == "/api/auth/passkey/authentication-options"
        || path == "/api/v1/auth/passkey/authentication-options"
        || path == "/api/auth/passkey/authentication-verify"
        || path == "/api/v1/auth/passkey/authentication-verify";

    if !is_auth_path {
        return next.run(request).await;
    }

    let ip = crate::client_ip::resolve_from_request(
        &request,
        &state.trusted_proxies,
        state.trust_forwarded_for,
    );

    // Loopback is exempt only when there is no upstream proxy.  A loopback
    // peer carrying any forwarding header indicates a reverse proxy on the
    // same host fronting public clients; those requests must still meter
    // (they share one bucket because the forwarded value is not trusted).
    // Without this guard, a same-host reverse-proxy deployment loses every
    // auth-attempt limit.
    if ip.is_loopback() && !has_forwarding_header(request.headers()) {
        return next.run(request).await;
    }

    if limiter.check_and_record(ip, max_attempts) {
        tracing::warn!(
            ip = %ip,
            path = %path,
            max_attempts,
            "Auth rate limit exceeded"
        );
        return rate_limit_response(
            "Too many login attempts. Please wait before trying again.",
            AUTH_RATE_LIMIT_RETRY_AFTER_SECS,
        );
    }

    next.run(request).await
}

// IP resolution lives in `crate::client_ip`. This module used to
// contain `resolve_client_ip` that ignored forwarding headers
// outright; the gating now happens through the shared
// `TrustedProxies` allowlist + `trust_forwarded_for` master switch
// passed via `GcraState` / `AuthRateLimitState`. An empty allowlist
// preserves the historical fail-closed behaviour exactly.

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use std::net::SocketAddr;
    use tower::ServiceExt;

    #[test]
    fn rate_limit_response_is_fail_closed() {
        let response = rate_limit_response("limited", 42);
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[header::RETRY_AFTER], "42");
    }

    /// Regression: a small-quota limiter must actually start rejecting
    /// after the burst is drained. Before this fix the nested-Result
    /// destructuring only caught `InsufficientCapacity`, so the inner
    /// `NotUntil` (the normal "out of tokens") path was treated as OK
    /// and `check_key_n` silently passed everything.
    #[test]
    fn test_rate_limit_trips_after_quota_drained() {
        let limiter = create_rate_limiter(5); // 5 tokens / minute
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let cost = NonZeroU32::new(1).unwrap();
        // Drain the burst (5 tokens) — these must all pass.
        for i in 0..5 {
            let r = limiter.check_key_n(&ip, cost);
            assert!(
                matches!(r, Ok(Ok(()))),
                "token {i} should pass but got {r:?}"
            );
        }
        // The next call must hit the inner NotUntil arm that the old
        // .is_err() missed. This is the precise shape the middleware
        // now pattern-matches on.
        let r = limiter.check_key_n(&ip, cost);
        assert!(
            matches!(r, Ok(Err(_))),
            "post-burst call must surface the NotUntil variant, got {r:?}"
        );
    }

    /// Regression for #3668: the GCRA limiter's DashMap grew unbounded
    /// because nothing wired up `retain_recent()` — every distinct client
    /// IP added a permanent entry. The fix in #3957 spawned a periodic
    /// sweep in `server.rs`. This test locks the contract the sweep
    /// depends on: `RateLimiter::len()` and `retain_recent()` must still
    /// be reachable on `KeyedRateLimiter`, and a fresh entry that has
    /// already drained back below the burst boundary must be evictable.
    ///
    /// Note: we cannot verify that `retain_recent()` *actually removes* a
    /// stale entry here because `KeyedRateLimiter` is pinned to
    /// `DefaultClock` (wall clock) and governor does not expose a way to
    /// advance it. Advancing time requires `FakeRelativeClock`, which
    /// produces a distinct type incompatible with `KeyedRateLimiter`. The
    /// time-based eviction path is covered by governor's own test suite.
    /// What this test guards is that the sweep's call chain
    /// (`Arc<KeyedRateLimiter> → retain_recent()`) compiles and runs
    /// without panicking, and that `retain_recent()` never *adds* entries.
    #[test]
    fn test_retain_recent_evicts_idle_entry() {
        let limiter = create_rate_limiter(60);
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let cost = NonZeroU32::new(1).unwrap();
        // Materialize an entry for `ip`.
        let _ = limiter.check_key_n(&ip, cost);
        assert!(
            !limiter.is_empty(),
            "check_key_n must register an entry, got len={}",
            limiter.len()
        );
        limiter.retain_recent();
        assert!(
            limiter.len() <= 1,
            "retain_recent must not duplicate entries, got len={}",
            limiter.len()
        );
    }

    #[test]
    fn test_static_assets_are_exempt() {
        // Root + common top-level assets.
        assert!(is_rate_limit_exempt("/"));
        assert!(is_rate_limit_exempt("/favicon.ico"));
        assert!(is_rate_limit_exempt("/logo.png"));
        // Dashboard SPA bundle and support files.
        assert!(is_rate_limit_exempt("/dashboard/index.html"));
        assert!(is_rate_limit_exempt("/dashboard/manifest.json"));
        assert!(is_rate_limit_exempt("/dashboard/sw.js"));
        assert!(is_rate_limit_exempt(
            "/dashboard/assets/ChatPage-ChE_yUYu.js"
        ));
        assert!(is_rate_limit_exempt("/dashboard/icon-192.png"));
        // Locale files loaded by the dashboard on boot.
        assert!(is_rate_limit_exempt("/locales/en.json"));
        assert!(is_rate_limit_exempt("/locales/ja.json"));
        assert!(is_rate_limit_exempt("/locales/uk.json"));
        assert!(is_rate_limit_exempt("/locales/zh-CN.json"));
        assert!(is_rate_limit_exempt("/locales/ko.json"));
    }

    #[test]
    fn test_metered_paths_are_not_exempt() {
        // Versioned + unversioned API.
        assert!(!is_rate_limit_exempt("/api/health"));
        assert!(!is_rate_limit_exempt("/api/v1/agents"));
        assert!(!is_rate_limit_exempt("/api/openapi.json"));
        assert!(!is_rate_limit_exempt("/api/versions"));
        // OpenAI-compatible layer, MCP, webhooks, channels — all must be
        // metered even though they live outside `/api/*`.
        assert!(!is_rate_limit_exempt("/v1/chat/completions"));
        assert!(!is_rate_limit_exempt("/v1/models"));
        assert!(!is_rate_limit_exempt("/mcp"));
        assert!(!is_rate_limit_exempt("/hooks/wake"));
        assert!(!is_rate_limit_exempt("/hooks/agent"));
        // Witness rotated from dingtalk → teams after the dingtalk
        // sidecar migration; the assertion is on the negative
        // prefix-discipline of the exempt list, not on dingtalk
        // specifically.
        assert!(!is_rate_limit_exempt("/channels/teams/webhook"));
        // Prefix discipline: the exempt list must not leak onto siblings.
        assert!(!is_rate_limit_exempt("/dashboard-login"));
        assert!(!is_rate_limit_exempt("/dashboardz"));
        assert!(!is_rate_limit_exempt("/localesX/en.json"));
    }

    fn router_with_limiter(tokens_per_minute: u32) -> Router {
        let state = GcraState {
            limiter: create_rate_limiter(tokens_per_minute),
            retry_after_secs: 60,
            trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
            trust_forwarded_for: false,
        };
        Router::new()
            .route("/dashboard/{*path}", get(|| async { "asset" }))
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state, gcra_rate_limit))
    }

    /// Build a request that carries an explicit `ConnectInfo` so the
    /// middleware sees the IP we want it to see. Without this, requests
    /// fall back to the unspecified-address default (`0.0.0.0`) and the
    /// loopback bypass added in #3416 doesn't trigger.
    fn request_from(uri: &str, ip: IpAddr) -> Request<Body> {
        let mut req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(SocketAddr::from((ip, 12345))));
        req
    }

    /// Same as [`request_from`] but also stamps `X-Forwarded-For` so
    /// the loopback bypass treats the peer as a same-host reverse
    /// proxy instead of a trusted local process.
    fn request_from_proxied(uri: &str, ip: IpAddr, xff_value: &str) -> Request<Body> {
        let mut req = request_from(uri, ip);
        req.headers_mut()
            .insert("x-forwarded-for", xff_value.parse().unwrap());
        req
    }

    /// Regression for the production 429 storm on `dash.librefang.ai`:
    /// a cold dashboard load fans out to ~20 static-asset requests, and
    /// the default fallback cost of 5 tokens drained the 500-token/min
    /// budget before the page finished rendering. With the exempt list
    /// in place, even a tiny budget must pass dashboard traffic through.
    #[tokio::test]
    async fn dashboard_burst_bypasses_rate_limit() {
        let app = router_with_limiter(1); // intentionally starved
        for i in 0..20 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/dashboard/manifest.json")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "dashboard request #{i} should bypass the limiter, got {:?}",
                resp.status()
            );
        }
    }

    /// Paired with the dashboard test above: the limiter *must* still
    /// bite on metered paths, otherwise the exempt list would be a
    /// blanket disable in disguise. Uses an RFC 5737 documentation IP
    /// (198.51.100.1) so the loopback bypass doesn't short-circuit it.
    #[tokio::test]
    async fn metered_api_burst_still_rate_limits() {
        let app = router_with_limiter(1);
        let public_ip: IpAddr = "198.51.100.1".parse().unwrap();
        let mut saw_429 = false;
        for _ in 0..20 {
            let resp = app
                .clone()
                .oneshot(request_from("/api/health", public_ip))
                .await
                .unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "metered /api/health burst must eventually hit 429 under a 1-token/min quota"
        );
    }

    /// Regression for #3416. With the limiter actually enforcing (after
    /// the `NotUntil` arm fix), a single dashboard tab on the same host
    /// drained the budget in seconds because every poll surfaces as
    /// 127.0.0.1. Loopback callers are local processes — there is no
    /// hostile-burst threat model — so they bypass the limiter outright.
    #[tokio::test]
    async fn loopback_v4_burst_bypasses_rate_limit() {
        let app = router_with_limiter(1); // intentionally starved
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        for i in 0..30 {
            let resp = app
                .clone()
                .oneshot(request_from("/api/health", loopback))
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "loopback request #{i} must bypass the limiter, got {:?}",
                resp.status()
            );
        }
    }

    /// IPv6 loopback (`::1`) is the same trust boundary as `127.0.0.1`
    /// — both surface for processes on the same host. Test the v6 case
    /// explicitly so a future refactor can't silently regress it.
    #[tokio::test]
    async fn loopback_v6_burst_bypasses_rate_limit() {
        let app = router_with_limiter(1);
        let loopback: IpAddr = "::1".parse().unwrap();
        for i in 0..30 {
            let resp = app
                .clone()
                .oneshot(request_from("/api/health", loopback))
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "loopback v6 request #{i} must bypass the limiter, got {:?}",
                resp.status()
            );
        }
    }

    /// Reverse-proxy guard: a loopback peer carrying `X-Forwarded-For`
    /// must NOT trigger the bypass. The peer is a same-host proxy
    /// fronting arbitrary public clients, not a trusted local process,
    /// so the limiter must still bite.
    #[tokio::test]
    async fn loopback_with_xff_does_not_bypass() {
        let app = router_with_limiter(1);
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let mut saw_429 = false;
        for _ in 0..20 {
            let resp = app
                .clone()
                .oneshot(request_from_proxied(
                    "/api/health",
                    loopback,
                    "203.0.113.42",
                ))
                .await
                .unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "loopback peer with X-Forwarded-For must still be rate-limited (proxy scenario)"
        );
    }

    /// Missing `ConnectInfo` (mis-wired middleware order) must NOT
    /// silently fail open through the loopback bypass. The fallback
    /// is `0.0.0.0`, which is non-loopback, so every such request
    /// enters the limiter and shares one bucket.
    #[tokio::test]
    async fn missing_connect_info_does_not_bypass() {
        let app = router_with_limiter(1);
        let mut saw_429 = false;
        for _ in 0..20 {
            // No ConnectInfo extension — simulates a mis-configured stack.
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/health")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "missing ConnectInfo must fall back to a non-loopback address and stay metered"
        );
    }

    #[test]
    fn test_has_forwarding_header_detects_common_variants() {
        let mut h = HeaderMap::new();
        assert!(!has_forwarding_header(&h));
        h.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
        assert!(has_forwarding_header(&h));
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "1.2.3.4".parse().unwrap());
        assert!(has_forwarding_header(&h));
        let mut h = HeaderMap::new();
        h.insert("forwarded", "for=1.2.3.4".parse().unwrap());
        assert!(has_forwarding_header(&h));
    }

    #[test]
    fn test_costs() {
        assert_eq!(operation_cost("GET", "/api/health").get(), 1);
        // Both spellings, kubelet-probe cost (#6633). See the comment on the
        // match arm for why the versioned form must not silently fall through
        // to the 5-token default.
        assert_eq!(operation_cost("GET", "/api/ready").get(), 1);
        assert_eq!(operation_cost("GET", "/api/v1/ready").get(), 1);
        assert_eq!(operation_cost("GET", "/api/tools").get(), 1);
        assert_eq!(operation_cost("POST", "/api/agents/1/message").get(), 30);
        assert_eq!(operation_cost("POST", "/api/agents").get(), 50);
        assert_eq!(operation_cost("POST", "/api/pairing/complete").get(), 50);
        assert_eq!(operation_cost("POST", "/api/workflows/1/run").get(), 100);
        assert_eq!(operation_cost("GET", "/api/agents/1/session").get(), 5);
        assert_eq!(operation_cost("GET", "/api/skills").get(), 2);
        assert_eq!(operation_cost("GET", "/api/peers").get(), 2);
        assert_eq!(operation_cost("GET", "/api/audit/recent").get(), 5);
        assert_eq!(operation_cost("POST", "/api/skills/install").get(), 50);
        assert_eq!(operation_cost("POST", "/api/migrate").get(), 100);
        // Dashboard high-frequency reads — kept at cost=1 so a polling
        // tab can't drain the budget. See #3416.
        assert_eq!(operation_cost("GET", "/api/dashboard/snapshot").get(), 1);
        assert_eq!(operation_cost("GET", "/api/approvals/count").get(), 1);
        assert_eq!(operation_cost("GET", "/api/providers").get(), 1);
        assert_eq!(operation_cost("GET", "/api/media/providers").get(), 1);
    }

    // ── Auth rate limiter unit tests ─────────────────────────────────────────

    #[test]
    fn auth_limiter_allows_within_budget() {
        let limiter = AuthLoginLimiter::new();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        // First 10 attempts must pass.
        for i in 0..10 {
            let rejected = limiter.check_and_record(ip, 10);
            assert!(!rejected, "attempt {i} should be allowed");
        }
    }

    #[test]
    fn auth_limiter_blocks_after_limit_exceeded() {
        let limiter = AuthLoginLimiter::new();
        let ip: IpAddr = "10.0.0.2".parse().unwrap();
        // Exhaust budget.
        for _ in 0..10 {
            limiter.check_and_record(ip, 10);
        }
        // 11th attempt must be rejected.
        let rejected = limiter.check_and_record(ip, 10);
        assert!(rejected, "11th attempt must be blocked");
    }

    #[test]
    fn auth_limiter_zero_max_disables() {
        let limiter = AuthLoginLimiter::new();
        let ip: IpAddr = "10.0.0.3".parse().unwrap();
        // With max_attempts=0 the limiter is disabled.
        for _ in 0..100 {
            let rejected = limiter.check_and_record(ip, 0);
            assert!(!rejected, "limiter should be a no-op when max_attempts=0");
        }
    }

    #[test]
    fn auth_limiter_different_ips_have_independent_buckets() {
        let limiter = AuthLoginLimiter::new();
        let ip_a: IpAddr = "10.0.0.4".parse().unwrap();
        let ip_b: IpAddr = "10.0.0.5".parse().unwrap();
        // Exhaust ip_a's budget.
        for _ in 0..10 {
            limiter.check_and_record(ip_a, 10);
        }
        // ip_b is unaffected.
        let rejected_b = limiter.check_and_record(ip_b, 10);
        assert!(!rejected_b, "ip_b must not be affected by ip_a's attempts");
        // ip_a is now blocked.
        let rejected_a = limiter.check_and_record(ip_a, 10);
        assert!(rejected_a, "ip_a must be blocked after exhausting budget");
    }

    #[test]
    fn auth_limiter_prune_removes_stale_entries() {
        let limiter = AuthLoginLimiter::new();
        let ip: IpAddr = "10.0.0.6".parse().unwrap();
        limiter.check_and_record(ip, 10);
        assert_eq!(limiter.map.len(), 1);
        // Prune with a zero cutoff: no entries should survive since all windows
        // started just now (elapsed < 30 minutes), so the map stays the same.
        // This test just ensures prune_stale() doesn't panic.
        limiter.prune_stale();
        // Entry must still be present (window_start is very recent).
        assert_eq!(limiter.map.len(), 1);
    }

    #[tokio::test]
    async fn auth_rate_limit_middleware_blocks_over_limit() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter: limiter.clone(),
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.10".parse().unwrap();

        let make_req = || {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55000,
                ))));
            req
        };

        // First two attempts must pass.
        for i in 0..2 {
            let resp = app.clone().oneshot(make_req()).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "attempt {i} must pass under the limit"
            );
        }
        // Third attempt must be rate-limited.
        let resp = app.oneshot(make_req()).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "3rd attempt must be blocked (limit=2)"
        );
        assert!(
            resp.headers().contains_key("retry-after"),
            "429 must include Retry-After header"
        );
    }

    #[tokio::test]
    async fn auth_rate_limit_middleware_loopback_exempt() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 1;
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        // Many loopback attempts must never trigger 429.
        for i in 0..20 {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    loopback, 55001,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "loopback attempt {i} must be exempt from the auth rate limiter"
            );
        }
    }

    /// Regression for audit `auth-callback-no-rate-limit`. The OAuth
    /// callback is public (the IdP redirects to it) but each
    /// successful HMAC verify consumes the `oauth_nonce_used` slot
    /// before the code exchange. A captured `state` token can be
    /// replayed from the open internet to lock the legitimate user
    /// out of completing their login. The path must be in the
    /// `is_auth_path` allowlist so per-IP rate limiting applies.
    #[tokio::test]
    async fn auth_rate_limit_middleware_blocks_oauth_callback_replay() {
        use axum::routing::{get, post};
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let app = Router::new()
            .route("/api/auth/callback", get(|| async { "ok" }))
            .route("/api/auth/callback", post(|| async { "ok" }))
            .route("/api/v1/auth/callback", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let attacker_ip: IpAddr = "203.0.113.42".parse().unwrap();

        let make_req = |uri: &str, method: axum::http::Method| {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    attacker_ip,
                    55050,
                ))));
            req
        };

        // First 2 GET callbacks pass (legitimate IdP redirect timing).
        for i in 0..2 {
            let resp = app
                .clone()
                .oneshot(make_req("/api/auth/callback", axum::http::Method::GET))
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "GET /api/auth/callback attempt {i} must pass under the limit"
            );
        }
        // 3rd attempt — replay floor — must be rejected.
        let resp = app
            .clone()
            .oneshot(make_req("/api/auth/callback", axum::http::Method::GET))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "3rd /api/auth/callback hit from the same IP must 429"
        );

        // /api/v1/auth/callback also counts (shares the limiter
        // bucket because it's the same handler under the alias).
        let resp = app
            .clone()
            .oneshot(make_req("/api/v1/auth/callback", axum::http::Method::GET))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "/api/v1/auth/callback must share the limiter bucket with /api/auth/callback",
        );

        // POST form of the callback also rate-limited (some IdPs
        // do form-POST instead of GET-with-query).
        let resp = app
            .oneshot(make_req("/api/auth/callback", axum::http::Method::POST))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "POST callback path also counts",
        );
    }

    #[tokio::test]
    async fn auth_rate_limit_middleware_non_auth_path_not_counted() {
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 1;
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.11".parse().unwrap();
        // Many hits to a non-auth path must never be rate-limited.
        for i in 0..20 {
            let mut req = Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55002,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "non-auth request #{i} must not be rate-limited"
            );
        }
    }

    /// Spoofing `X-Forwarded-For` per request must NOT bypass the limit.
    /// The limiter keys on `peer_addr` only; rotating the header value
    /// each request keeps `peer_addr` constant, so the bucket fills.
    /// This is exactly the bypass pattern flagged in the post-merge
    /// audit of #3950 — without this regression test it can return.
    #[tokio::test]
    async fn auth_rate_limit_xff_spoof_does_not_bypass() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let attacker_peer: IpAddr = "203.0.113.99".parse().unwrap();
        let mut saw_429 = false;
        for i in 0..10 {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .body(Body::empty())
                .unwrap();
            // Rotate X-Forwarded-For to a fresh fake IP each request —
            // the spoof an internet attacker would actually use.
            req.headers_mut()
                .insert("x-forwarded-for", format!("1.2.3.{i}").parse().unwrap());
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    attacker_peer,
                    55003,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "rotating X-Forwarded-For per request must not bypass the per-IP limit"
        );
    }

    /// A loopback peer carrying any forwarding header is a same-host
    /// reverse-proxy fronting public clients — must NOT be exempt.
    #[tokio::test]
    async fn auth_rate_limit_loopback_with_xff_not_exempt() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 1;
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let mut saw_429 = false;
        for _ in 0..10 {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .body(Body::empty())
                .unwrap();
            req.headers_mut()
                .insert("x-forwarded-for", "203.0.113.42".parse().unwrap());
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    loopback, 55004,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "loopback peer with a forwarding header must still be rate-limited"
        );
    }

    /// TOTP confirm and approval endpoints accept 6-digit / recovery
    /// codes; they must be in the rate-limited path set so an attacker
    /// who already cleared the password gate cannot brute-force codes.
    #[tokio::test]
    async fn auth_rate_limit_covers_totp_and_approval_endpoints() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let public_ip: IpAddr = "203.0.113.55".parse().unwrap();

        for path in &[
            "/api/approvals/some-id/approve",
            "/api/v1/approvals/some-id/approve",
            "/api/approvals/totp/confirm",
            "/api/v1/approvals/totp/confirm",
        ] {
            let limiter = Arc::new(AuthLoginLimiter::new());
            let max_attempts: u32 = 1;
            // `true`: the condition this test asserts is the one where the
            // policy does require a code, which is the only case in which an
            // approve path has a credential to protect. The path is
            // deliberately not metered otherwise — see
            // `approvals_do_not_share_the_login_bucket_when_no_totp_is_verified`.
            let app = Router::new().route(path, post(|| async { "ok" })).layer(
                axum::middleware::from_fn_with_state(
                    auth_state(limiter, max_attempts, true),
                    auth_rate_limit_layer,
                ),
            );

            let mut saw_429 = false;
            for _ in 0..5 {
                let mut req = Request::builder()
                    .method("POST")
                    .uri(*path)
                    .body(Body::empty())
                    .unwrap();
                req.extensions_mut()
                    .insert(axum::extract::ConnectInfo(SocketAddr::from((
                        public_ip, 55005,
                    ))));
                let resp = app.clone().oneshot(req).await.unwrap();
                if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                    saw_429 = true;
                    break;
                }
            }
            assert!(saw_429, "endpoint {path} must be rate-limited but was not");
        }
    }

    /// When the daemon is configured to trust a reverse proxy
    /// (`trusted_proxies` non-empty + `trust_forwarded_for=true`), a
    /// request arriving from the trusted peer with `X-Forwarded-For`
    /// must key on the forwarded IP, not the proxy's address. This is
    /// the change PR'd in for trusted-proxies support: previously every
    /// browser behind cloudflared collapsed onto one shared bucket.
    #[tokio::test]
    async fn auth_rate_limit_keys_on_forwarded_ip_when_proxy_trusted() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let trusted = Arc::new(crate::client_ip::TrustedProxies::compile(&[
            "172.19.0.0/16".to_string(),
        ]));
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter: limiter.clone(),
                    max_attempts,
                    trusted_proxies: trusted,
                    trust_forwarded_for: true,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let proxy_peer: IpAddr = "172.19.0.1".parse().unwrap();
        let browser_a: &str = "203.0.113.10";
        let browser_b: &str = "203.0.113.20";

        let make_req = |xff: &str| {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .header("x-forwarded-for", xff)
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    proxy_peer, 55000,
                ))));
            req
        };

        // Browser A burns its budget.
        for _ in 0..2 {
            let r = app.clone().oneshot(make_req(browser_a)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
        }
        let r = app.clone().oneshot(make_req(browser_a)).await.unwrap();
        assert_eq!(
            r.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "browser A is over the limit on its own bucket"
        );

        // Browser B (same proxy peer, different XFF) gets a fresh bucket.
        let r = app.clone().oneshot(make_req(browser_b)).await.unwrap();
        assert_eq!(
            r.status(),
            StatusCode::OK,
            "browser B must NOT inherit browser A's exhausted bucket"
        );
    }

    /// Spoof regression: an internet client (peer NOT in
    /// `trusted_proxies`) that rotates `X-Forwarded-For` to a fresh
    /// fake IP per request must NOT bypass the limiter. Forwarding
    /// headers from untrusted peers are ignored entirely; the limiter
    /// keys on the actual TCP source.
    #[tokio::test]
    async fn auth_rate_limit_ignores_xff_from_untrusted_peer() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let trusted = Arc::new(crate::client_ip::TrustedProxies::compile(&[
            // Only the internal proxy subnet is trusted. The attacker's
            // real source (198.51.100.7 below) does not match.
            "172.19.0.0/16".to_string(),
        ]));
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: trusted,
                    trust_forwarded_for: true,
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let attacker_peer: IpAddr = "198.51.100.7".parse().unwrap();

        let make_req = |fake_xff: &str| {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .header("x-forwarded-for", fake_xff)
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    attacker_peer,
                    44444,
                ))));
            req
        };

        // First two attempts pass (counting against attacker_peer).
        for _ in 0..2 {
            let r = app.clone().oneshot(make_req("10.0.0.1")).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
        }
        // Third attempt with a fresh forged XFF must STILL 429 — the
        // forged header is ignored because the peer is not trusted.
        let r = app.clone().oneshot(make_req("10.0.0.99")).await.unwrap();
        assert_eq!(
            r.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "rotating X-Forwarded-For from an untrusted peer must not bypass the limiter"
        );
    }

    /// When `trust_forwarded_for` is false (default), even a peer that
    /// matches `trusted_proxies` is not allowed to set the client IP
    /// via headers. The master switch is the kill-switch.
    #[tokio::test]
    async fn auth_rate_limit_ignores_xff_when_master_flag_off() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let trusted = Arc::new(crate::client_ip::TrustedProxies::compile(&[
            "172.19.0.0/16".to_string(),
        ]));
        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter,
                    max_attempts,
                    trusted_proxies: trusted,
                    trust_forwarded_for: false, // master switch off
                    approvals_require_totp: Arc::new(|_| false),
                },
                auth_rate_limit_layer,
            ));

        let proxy_peer: IpAddr = "172.19.0.1".parse().unwrap();

        let make_req = |xff: &str| {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .header("x-forwarded-for", xff)
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    proxy_peer, 55000,
                ))));
            req
        };

        // Two unique XFFs burn the SAME bucket (proxy_peer's), because
        // the master switch is off and headers are ignored.
        for _ in 0..2 {
            let r = app.clone().oneshot(make_req("203.0.113.10")).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
        }
        let r = app.oneshot(make_req("203.0.113.99")).await.unwrap();
        assert_eq!(
            r.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "master switch off → XFF must be ignored, all requests share peer's bucket"
        );
    }

    /// Build the per-IP auth-limiter state for the middleware tests below.
    ///
    /// `approvals_require_totp` stands in for the live per-request answer:
    /// `true` when `approve_request` would verify a 6-digit code for the tool
    /// behind the path's approval id and the caller is outside the grace
    /// window, `false` when it would verify nothing. These tests fix one
    /// answer for every id; `approve_gate_consults_the_predicate_with_the_requests_own_id`
    /// covers the id-by-id case.
    fn auth_state(
        limiter: Arc<AuthLoginLimiter>,
        max_attempts: u32,
        approvals_require_totp: bool,
    ) -> AuthRateLimitState {
        AuthRateLimitState {
            limiter,
            max_attempts,
            trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
            trust_forwarded_for: false,
            approvals_require_totp: Arc::new(move |_approval_id: &str| approvals_require_totp),
        }
    }

    /// Regression: an operator approving a batch of tool calls must not be
    /// locked out by the login bucket while the daemon verifies no code on
    /// that path.
    ///
    /// `approve_request` demands a TOTP or recovery code only when the
    /// approval policy requires one — `ApprovalPolicy::tool_requires_totp` is
    /// false for every tool while `second_factor` is `none` (the default) or
    /// `login`, and short-circuits to false in that case. With no code to
    /// guess there is no brute-force surface, yet the middleware counted every
    /// *successful* approval against the same 10-per-15-minute bucket it shares
    /// with `dashboard-login`: the eleventh approval in a quarter of an hour
    /// answered 429 "Too many login attempts" and locked the operator out for a
    /// further 15 minutes.
    #[tokio::test]
    async fn approvals_do_not_share_the_login_bucket_when_no_totp_is_verified() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let app = Router::new()
            .route("/api/approvals/some-id/approve", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                auth_state(limiter.clone(), max_attempts, false),
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.31".parse().unwrap();

        // Two attempts past the cap: if the path still shares the login
        // bucket these are 429s, which is the reported lockout.
        for i in 0..(max_attempts + 2) {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/approvals/some-id/approve")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55000,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "approval {i} must not be counted against the login bucket while \
                 no TOTP code is verified (limit={max_attempts})"
            );
        }
    }

    /// The #4020 protection, kept: while the policy requires TOTP for
    /// approvals the approve path verifies a 6-digit code, so it stays metered
    /// alongside `dashboard-login`.
    #[tokio::test]
    async fn approvals_share_the_login_bucket_when_totp_is_verified() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let limiter = Arc::new(AuthLoginLimiter::new());
        let max_attempts: u32 = 2;
        let app = Router::new()
            .route("/api/approvals/some-id/approve", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                auth_state(limiter.clone(), max_attempts, true),
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.32".parse().unwrap();

        let mut saw_429 = false;
        for _ in 0..(max_attempts + 2) {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/approvals/some-id/approve")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55000,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "approve must stay rate-limited while the policy requires a TOTP code (limit={max_attempts})"
        );
    }

    /// The gate must be asked about the approval id carried by the path, not
    /// answered from a policy-wide constant: with `totp_tools` narrowed, one
    /// id verifies a code and another does not, and the bucket must follow.
    #[tokio::test]
    async fn approve_gate_consults_the_predicate_with_the_requests_own_id() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let max_attempts: u32 = 1;
        let asked: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let asked_by_middleware = Arc::clone(&asked);
        let app = Router::new()
            .route("/api/approvals/{id}/approve", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter: Arc::new(AuthLoginLimiter::new()),
                    max_attempts,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    // Only the listed id has a code to verify.
                    approvals_require_totp: Arc::new(move |approval_id: &str| {
                        asked_by_middleware
                            .lock()
                            .unwrap()
                            .push(approval_id.to_string());
                        approval_id == "verify-me"
                    }),
                },
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.61".parse().unwrap();
        let make_req = |id: &str| {
            let mut req = Request::builder()
                .method("POST")
                .uri(format!("/api/approvals/{id}/approve"))
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55000,
                ))));
            req
        };

        // The id with nothing to verify never spends the bucket, however many
        // times it is sent.
        for _ in 0..(max_attempts + 2) {
            let resp = app.clone().oneshot(make_req("free-id")).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "an approval id that verifies no code must not be counted"
            );
        }

        // The id that verifies a code does: the request past the cap trips.
        let mut saw_429 = false;
        for _ in 0..=(max_attempts + 1) {
            let resp = app.clone().oneshot(make_req("verify-me")).await.unwrap();
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "the approval id with a code to verify must still be metered"
        );

        let asked = asked.lock().unwrap();
        assert!(
            asked.iter().any(|id| id == "free-id") && asked.iter().any(|id| id == "verify-me"),
            "the predicate must be asked about each request's own id, saw {asked:?}"
        );
        assert!(
            asked.iter().all(|id| id != "approve"),
            "the id segment must not include the `/approve` suffix, saw {asked:?}"
        );
    }

    /// The predicate takes the kernel's approval-policy lock, so it must be
    /// consulted only on the approve path. A predicate that panics when called
    /// proves it: every non-approve route below would abort the request if the
    /// middleware asked before deciding the path is not an approve one.
    #[tokio::test]
    async fn approve_gate_predicate_is_not_asked_off_the_approve_path() {
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new()
            .route("/api/auth/dashboard-login", post(|| async { "ok" }))
            .route("/api/approvals/totp/confirm", post(|| async { "ok" }))
            .route("/api/approvals/some-id/reject", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter: Arc::new(AuthLoginLimiter::new()),
                    max_attempts: 5,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    approvals_require_totp: Arc::new(|_approval_id: &str| {
                        panic!("the approve gate must not be consulted for other routes")
                    }),
                },
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.62".parse().unwrap();
        for uri in [
            "/api/auth/dashboard-login",
            "/api/approvals/totp/confirm",
            "/api/approvals/some-id/reject",
        ] {
            let mut req = Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55000,
                ))));
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "{uri} must not consult the approve gate"
            );
        }
    }

    /// Low-level coverage for [`approve_path_request_id`]: the versioned
    /// prefix, the shapes that are not an approve path, and the empty /
    /// multi-segment id that must not reach the gate.
    #[test]
    fn approve_path_request_id_matches_only_single_segment_approve_paths() {
        assert_eq!(
            approve_path_request_id("/api/approvals/abc/approve").as_deref(),
            Some("abc")
        );
        assert_eq!(
            approve_path_request_id("/api/v1/approvals/abc/approve").as_deref(),
            Some("abc"),
            "the versioned prefix must be recognised"
        );
        assert_eq!(
            approve_path_request_id("/api/approvals//approve").as_deref(),
            None,
            "an empty id must not be passed to the gate"
        );
        assert_eq!(
            approve_path_request_id("/api/approvals/a/b/approve").as_deref(),
            None,
            "an id that spans segments must not be passed to the gate"
        );
        assert_eq!(
            approve_path_request_id("/api/approvals/abc/reject").as_deref(),
            None,
            "only the /approve suffix counts"
        );
        assert_eq!(
            approve_path_request_id("/api/approvals/abc/approve/").as_deref(),
            None,
            "a trailing slash after the suffix is not an approve path"
        );
    }

    /// The middleware reads `request.uri().path()` (raw, percent-encoded)
    /// while `approve_request` binds `Path<String>` (decoded). A UUID whose
    /// hyphens arrive escaped as `%2D` must resolve to the same id on both
    /// ends, or the request that verifies a code is the one left unmetered.
    #[test]
    fn approve_path_request_id_decodes_percent_escapes_like_the_handler() {
        let canonical = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        let encoded = canonical.replacen('-', "%2D", 1);
        assert_ne!(encoded, canonical);
        assert!(
            uuid::Uuid::parse_str(&encoded).is_err(),
            "the raw segment must not parse as a UUID — the pre-fix failure mode"
        );

        let path = format!("/api/approvals/{encoded}/approve");
        let parsed = approve_path_request_id(&path)
            .expect("a percent-encoded approve path must be recognised");
        assert_eq!(parsed.as_ref(), canonical);
        assert_eq!(
            uuid::Uuid::parse_str(parsed.as_ref()).unwrap(),
            uuid::Uuid::parse_str(canonical).unwrap(),
            "the decoded id must parse to the UUID the handler resolves"
        );
    }

    /// End-to-end version of the parity above: with the route's `{id}` bound
    /// by the real `Path<String>` extractor, a `%2D` id must be metered (the
    /// gate is asked, and the bucket trips) and both the middleware and the
    /// handler must see the same decoded UUID.
    #[tokio::test]
    async fn approve_gate_meters_percent_encoded_ids_the_handler_binds() {
        use axum::extract::Path;
        use axum::routing::post;
        use axum::Router;
        use tower::ServiceExt;

        let canonical = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        let encoded = canonical.replacen('-', "%2D", 1);

        let seen_by_middleware: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
        let seen_by_handler: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
        let gate_saw = Arc::clone(&seen_by_middleware);
        let handler_saw = Arc::clone(&seen_by_handler);

        let app = Router::new()
            .route(
                "/api/approvals/{id}/approve",
                post(move |Path(id): Path<String>| {
                    let seen = Arc::clone(&handler_saw);
                    async move {
                        *seen.lock().unwrap() = Some(id);
                        "ok"
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                AuthRateLimitState {
                    limiter: Arc::new(AuthLoginLimiter::new()),
                    max_attempts: 1,
                    trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
                    trust_forwarded_for: false,
                    // Same shape as the production wiring: parse the id, then
                    // ask whether approving it would verify a code. Here the
                    // answer is "yes" for any UUID.
                    approvals_require_totp: Arc::new(move |approval_id: &str| {
                        *gate_saw.lock().unwrap() = Some(approval_id.to_string());
                        uuid::Uuid::parse_str(approval_id).is_ok()
                    }),
                },
                auth_rate_limit_layer,
            ));

        let public_ip: IpAddr = "203.0.113.63".parse().unwrap();
        let make_req = || {
            let mut req = Request::builder()
                .method("POST")
                .uri(format!("/api/approvals/{encoded}/approve"))
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    public_ip, 55000,
                ))));
            req
        };

        let resp = app.clone().oneshot(make_req()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // max_attempts = 1: the second request from the same IP must trip the
        // limit. Before the decode fix the gate answered `false` for the
        // encoded id and this stayed green forever.
        let resp = app.clone().oneshot(make_req()).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "the encoded approve path must spend the login bucket like the plain form"
        );

        assert_eq!(
            seen_by_middleware.lock().unwrap().as_deref(),
            Some(canonical),
            "the middleware must ask the gate about the decoded id"
        );
        assert_eq!(
            seen_by_handler.lock().unwrap().as_deref(),
            Some(canonical),
            "the handler must bind the same decoded id"
        );
    }
}
