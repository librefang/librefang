//! Cost-aware rate limiting using GCRA (Generic Cell Rate Algorithm).
//!
//! Each API operation has a token cost (e.g., health=1, spawn=50, message=30).
//! The GCRA algorithm allows 500 tokens per minute per IP address.
//!
//! Non-API paths (dashboard SPA assets, locale JSON, favicon, logo, root) are
//! exempt from rate limiting — a single dashboard page load fans out to dozens
//! of static-asset requests, and accounting them at the default fallback cost
//! drains the budget before the page finishes rendering. See
//! [`is_rate_limit_exempt`].

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::middleware::Next;
use governor::{clock::DefaultClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

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
        ("POST", p) if p.contains("/message") => NonZeroU32::new(30).unwrap(),
        ("POST", p) if p.contains("/run") => NonZeroU32::new(100).unwrap(),
        ("POST", "/api/skills/install") => NonZeroU32::new(50).unwrap(),
        ("POST", "/api/skills/uninstall") => NonZeroU32::new(10).unwrap(),
        ("POST", "/api/migrate") => NonZeroU32::new(100).unwrap(),
        ("PUT", p) if p.contains("/update") => NonZeroU32::new(10).unwrap(),
        _ => NonZeroU32::new(5).unwrap(),
    }
}

pub type KeyedRateLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

/// Shared state for the GCRA rate limiting middleware layer.
#[derive(Clone)]
pub struct GcraState {
    pub limiter: Arc<KeyedRateLimiter>,
    pub retry_after_secs: u64,
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

    let ip = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));

    // Loopback (127.0.0.0/8 + ::1) bypasses the limiter. The dashboard
    // SPA, the librefang CLI, and any other process on the same host
    // talking to its own daemon all surface as loopback, and there's no
    // hostile-burst threat model from a peer that already has local
    // process privileges. Without this, a single dashboard tab refresh
    // (snapshot + approvals/count + providers + media/providers + …,
    // re-fetched on focus + interval) drained the 500-token/min budget
    // in seconds and 429'd the whole UI. See #3416.
    if ip.is_loopback() {
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
        let retry_after = state.retry_after_secs.to_string();
        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("content-type", "application/json")
            .header("retry-after", retry_after)
            .body(Body::from(
                serde_json::json!({"error": "Rate limit exceeded"}).to_string(),
            ))
            .unwrap_or_default();
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

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
        assert!(is_rate_limit_exempt("/locales/zh-CN.json"));
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
        assert!(!is_rate_limit_exempt("/channels/feishu/webhook"));
        // Prefix discipline: the exempt list must not leak onto siblings.
        assert!(!is_rate_limit_exempt("/dashboard-login"));
        assert!(!is_rate_limit_exempt("/dashboardz"));
        assert!(!is_rate_limit_exempt("/localesX/en.json"));
    }

    fn router_with_limiter(tokens_per_minute: u32) -> Router {
        let state = GcraState {
            limiter: create_rate_limiter(tokens_per_minute),
            retry_after_secs: 60,
        };
        Router::new()
            .route("/dashboard/{*path}", get(|| async { "asset" }))
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state, gcra_rate_limit))
    }

    /// Build a request that carries an explicit `ConnectInfo` so the
    /// middleware sees the IP we want it to see. Without this, requests
    /// fall back to the loopback default and skip the limiter entirely
    /// after #3416.
    fn request_from(uri: &str, ip: IpAddr) -> Request<Body> {
        let mut req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(SocketAddr::from((ip, 12345))));
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

    #[test]
    fn test_costs() {
        assert_eq!(operation_cost("GET", "/api/health").get(), 1);
        assert_eq!(operation_cost("GET", "/api/tools").get(), 1);
        assert_eq!(operation_cost("POST", "/api/agents/1/message").get(), 30);
        assert_eq!(operation_cost("POST", "/api/agents").get(), 50);
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
}
