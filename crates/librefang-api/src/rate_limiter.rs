//! Cost-aware rate limiting using GCRA (Generic Cell Rate Algorithm).
//!
//! Each API operation has a token cost (e.g., health=1, spawn=50, message=30).
//! The GCRA algorithm allows 500 tokens per minute per tenant account when
//! available, with anonymous traffic falling back to per-IP limiting.

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::middleware::Next;
use governor::{clock::DefaultClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum RateLimitKey {
    Account(String),
    AnonymousIp(IpAddr),
}

pub fn operation_cost(method: &str, path: &str) -> NonZeroU32 {
    match (method, path) {
        (_, "/api/health") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/status") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/version") => NonZeroU32::new(1).unwrap(),
        ("GET", "/api/tools") => NonZeroU32::new(1).unwrap(),
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

pub type KeyedRateLimiter =
    RateLimiter<RateLimitKey, DashMapStateStore<RateLimitKey>, DefaultClock>;

/// Shared state for the GCRA rate limiting middleware layer.
#[derive(Clone)]
pub struct GcraState {
    pub limiter: Arc<KeyedRateLimiter>,
    pub retry_after_secs: u64,
}

/// Create a GCRA rate limiter with the given token budget per minute per tenant
/// account, with anonymous traffic falling back to per-IP keys.
pub fn create_rate_limiter(tokens_per_minute: u32) -> Arc<KeyedRateLimiter> {
    let quota = tokens_per_minute.max(1);
    Arc::new(RateLimiter::keyed(Quota::per_minute(
        NonZeroU32::new(quota).unwrap(),
    )))
}

fn rate_limit_key(headers: &axum::http::HeaderMap, ip: IpAddr) -> RateLimitKey {
    headers
        .get("x-account-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|account_id| RateLimitKey::Account(account_id.to_string()))
        .unwrap_or(RateLimitKey::AnonymousIp(ip))
}

/// GCRA rate limiting middleware.
///
/// Extracts the client IP from `ConnectInfo`, computes the cost for the
/// requested operation, and checks the GCRA limiter. Returns 429 if the
/// client has exhausted its token budget.
pub async fn gcra_rate_limit(
    axum::extract::State(state): axum::extract::State<GcraState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let ip = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let key = rate_limit_key(request.headers(), ip);
    let account_id = match &key {
        RateLimitKey::Account(account_id) => Some(account_id.clone()),
        RateLimitKey::AnonymousIp(_) => None,
    };

    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let cost = operation_cost(&method, &path);

    if state.limiter.check_key_n(&key, cost).is_err() {
        tracing::warn!(
            ip = %ip,
            account_id = ?account_id,
            cost = cost.get(),
            path = %path,
            "GCRA rate limit exceeded"
        );
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
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_ip_different_accounts_do_not_share_rate_limit_bucket() {
        let limiter = create_rate_limiter(1);
        let cost = NonZeroU32::new(1).unwrap();
        assert!(limiter
            .check_key_n(&RateLimitKey::Account("tenant-a".to_string()), cost)
            .is_ok());
        assert!(limiter
            .check_key_n(&RateLimitKey::Account("tenant-b".to_string()), cost)
            .is_ok());
    }

    #[test]
    fn rate_limit_key_uses_account_id_when_present() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-account-id", "tenant-a".parse().unwrap());
        let key = rate_limit_key(&headers, IpAddr::from([127, 0, 0, 1]));
        assert_eq!(key, RateLimitKey::Account("tenant-a".to_string()));
    }

    #[test]
    fn rate_limit_key_falls_back_to_ip_when_account_missing() {
        let headers = axum::http::HeaderMap::new();
        let ip = IpAddr::from([127, 0, 0, 1]);
        let key = rate_limit_key(&headers, ip);
        assert_eq!(key, RateLimitKey::AnonymousIp(ip));
    }
}
