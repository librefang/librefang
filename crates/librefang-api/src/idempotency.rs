//! Idempotency-Key replay middleware for state-creating POSTs (#3637).
//!
//! Opt-in: callers signal "this request is replay-safe" by sending an
//! `Idempotency-Key: <opaque-string>` header. When set, the handler runs
//! through [`run_idempotent`], which:
//!
//! 1. Looks up `(key)` in the persistent store.
//! 2. **Cache miss**: executes the inner handler, then persists the
//!    successful 2xx response under `(key, body_hash)` for 24 hours.
//!    Non-2xx responses are not cached so a transient failure (rate
//!    limit, downstream blip) does not poison the slot — clients can
//!    retry the same key and get a real attempt.
//! 3. **Cache hit, same body**: replays the cached `(status, body)`
//!    without re-executing the handler.
//! 4. **Cache hit, different body**: returns 409 Conflict. The
//!    `Idempotency-Key` is the operator-supplied dedup token and a
//!    different payload under the same key is a programming error
//!    (e.g. UI accidentally reuses an old key after editing the form).
//!
//! Body identity is sha256 over the raw JSON bytes the handler
//! received. We hash bytes, not parsed JSON, so a re-serialised body
//! with reordered keys would mismatch — that's the safer default;
//! callers that want canonicalisation can do it before sending.
//!
//! The persistent store lives in `librefang-memory` so the API crate
//! stays free of `rusqlite`. Production wires
//! `SqliteIdempotencyStore` against the substrate connection at boot.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use librefang_memory::idempotency::{CachedResponse, IdempotencyStore};
use sha2::{Digest, Sha256};

/// Maximum length of a client-supplied `Idempotency-Key`. Bounded so a
/// pathological client cannot bloat the SQLite primary key — UUIDs,
/// ULIDs, hex digests and Stripe-style hyphenated tokens fit
/// comfortably.
pub const MAX_KEY_LEN: usize = 255;

/// HTTP header name carrying the operator-supplied key.
pub const HEADER_NAME: &str = "Idempotency-Key";

/// Hash request bytes for body-conflict detection. Hex-encoded sha256.
pub fn hash_body(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        use std::fmt::Write as _;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Validate an inbound `Idempotency-Key` header value.
///
/// Empty / oversize / non-printable values are rejected so we never
/// store garbage as the primary key. We accept ASCII printable
/// (33..=126) — UUIDs, base64, hex, ULIDs, and Stripe-style
/// hyphenated tokens all fit.
pub fn validate_key(raw: &str) -> Result<&str, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Idempotency-Key must not be empty");
    }
    if trimmed.len() > MAX_KEY_LEN {
        return Err("Idempotency-Key exceeds 255 bytes");
    }
    if !trimmed.bytes().all(|b| (33..=126).contains(&b)) {
        return Err("Idempotency-Key must be printable ASCII");
    }
    Ok(trimmed)
}

/// 409 Conflict body returned when a key was reused with a different payload.
pub fn body_conflict_response() -> Response {
    let payload = serde_json::json!({
        "error": "Idempotency-Key was reused with a different request body",
        "code": "idempotency_key_conflict",
        "type": "idempotency_key_conflict",
    });
    (StatusCode::CONFLICT, Json(payload)).into_response()
}

/// Wrap a handler closure with idempotency semantics.
///
/// `key_header` is `None` when the caller did not send an
/// `Idempotency-Key` header — we just run `f` and pass the response
/// through (the caller-as-of-today path). With a header set, we go
/// through the cache.
///
/// The handler closure returns `(status, body_bytes)`. We choose this
/// shape (instead of `Response`) because the API layer's handlers
/// already build JSON values; serialising to bytes is the cheapest
/// way to capture a replayable snapshot, and lets us round-trip
/// through SQLite without needing to clone an `axum::Response`.
pub async fn run_idempotent<F, Fut>(
    store: &dyn IdempotencyStore,
    key_header: Option<&str>,
    body_bytes: &[u8],
    f: F,
) -> Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = (StatusCode, Vec<u8>)>,
{
    // Fast path: no header → no caching, original behaviour preserved.
    let raw_key = match key_header {
        Some(k) => k,
        None => {
            let (status, body) = f().await;
            return build_response(status, body);
        }
    };

    let key = match validate_key(raw_key) {
        Ok(k) => k,
        Err(msg) => {
            let payload = serde_json::json!({
                "error": msg,
                "code": "idempotency_key_invalid",
                "type": "idempotency_key_invalid",
            });
            return (StatusCode::BAD_REQUEST, Json(payload)).into_response();
        }
    };

    let body_hash = hash_body(body_bytes);

    // Lookup. On lookup error we degrade to "execute anyway, don't
    // cache" so a corrupt cache row can never block real traffic.
    let prior = match store.lookup(key) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "idempotency lookup failed; bypassing cache");
            let (status, body) = f().await;
            return build_response(status, body);
        }
    };

    if let Some(existing) = prior {
        if existing.body_hash != body_hash {
            return body_conflict_response();
        }
        // Same key, same body → replay.
        return build_response(
            StatusCode::from_u16(existing.response.status)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            existing.response.body,
        );
    }

    // Miss — execute the handler. Cache only successful 2xx responses;
    // 4xx/5xx remain retriable so a transient failure does not poison
    // the slot.
    let (status, body) = f().await;
    if status.is_success() {
        let cached = CachedResponse {
            status: status.as_u16(),
            body: body.clone(),
        };
        if let Err(e) = store.put(key, &body_hash, &cached) {
            tracing::warn!(error = %e, "idempotency persist failed; response returned without caching");
        }
    }
    // Opportunistic prune so the table self-trims.
    if let Err(e) = store.prune_expired() {
        tracing::debug!(error = %e, "idempotency prune_expired failed");
    }

    build_response(status, body)
}

fn build_response(status: StatusCode, body: Vec<u8>) -> Response {
    use axum::body::Body;
    use axum::http::header;
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "response build failed").into_response()
        })
}

/// Read the `Idempotency-Key` header from a request map, returning
/// `None` if absent or non-UTF-8 (rejecting non-UTF-8 here is fine —
/// `validate_key` would also reject the value).
pub fn extract_key(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_memory::idempotency::{IdempotencyError, StoredRecord};
    use std::sync::Mutex;

    /// In-memory stub so unit tests exercise the middleware without
    /// any SQLite dependency in `librefang-api` itself. End-to-end
    /// SQLite coverage lives in `tests/idempotency_test.rs` (which
    /// goes through the real `start_test_server` harness) and in
    /// `librefang-memory`'s own unit tests.
    #[derive(Default)]
    struct MemStore {
        rows: Mutex<std::collections::HashMap<String, StoredRecord>>,
    }
    impl IdempotencyStore for MemStore {
        fn lookup(&self, key: &str) -> Result<Option<StoredRecord>, IdempotencyError> {
            Ok(self.rows.lock().unwrap().get(key).cloned())
        }
        fn put(
            &self,
            key: &str,
            body_hash: &str,
            response: &CachedResponse,
        ) -> Result<(), IdempotencyError> {
            self.rows
                .lock()
                .unwrap()
                .entry(key.to_string())
                .or_insert_with(|| StoredRecord {
                    body_hash: body_hash.to_string(),
                    response: response.clone(),
                });
            Ok(())
        }
        fn prune_expired(&self) -> Result<(), IdempotencyError> {
            Ok(())
        }
    }

    #[test]
    fn validate_key_rejects_empty_and_oversize() {
        assert!(validate_key("").is_err());
        assert!(validate_key("   ").is_err());
        let big = "a".repeat(MAX_KEY_LEN + 1);
        assert!(validate_key(&big).is_err());
        assert!(validate_key("good-key-123").is_ok());
    }

    #[tokio::test]
    async fn run_idempotent_no_header_skips_cache() {
        let s = MemStore::default();
        let counter = std::sync::atomic::AtomicUsize::new(0);
        let body = b"{}".to_vec();
        let r = run_idempotent(&s, None, &body, || async {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::CREATED, b"{\"ok\":true}".to_vec())
        })
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Without a key, nothing is persisted.
        assert!(s.lookup("anything").unwrap().is_none());
    }

    #[tokio::test]
    async fn run_idempotent_replays_same_body() {
        let s = MemStore::default();
        let body = b"{\"x\":1}".to_vec();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mk = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::CREATED, b"{\"id\":\"agent-1\"}".to_vec())
        };
        let r1 = run_idempotent(&s, Some("dup-key"), &body, mk).await;
        assert_eq!(r1.status(), StatusCode::CREATED);
        let r2 = run_idempotent(&s, Some("dup-key"), &body, mk).await;
        assert_eq!(r2.status(), StatusCode::CREATED);
        // Inner handler ran exactly once.
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn run_idempotent_conflict_on_different_body() {
        let s = MemStore::default();
        let r1 = run_idempotent(&s, Some("k"), b"{\"a\":1}", || async {
            (StatusCode::CREATED, b"first".to_vec())
        })
        .await;
        assert_eq!(r1.status(), StatusCode::CREATED);
        let r2 = run_idempotent(&s, Some("k"), b"{\"a\":2}", || async {
            (StatusCode::CREATED, b"second".to_vec())
        })
        .await;
        assert_eq!(r2.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn non_2xx_responses_are_not_cached() {
        let s = MemStore::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mk_fail = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::INTERNAL_SERVER_ERROR, b"boom".to_vec())
        };
        let r1 = run_idempotent(&s, Some("retry-me"), b"{}", mk_fail).await;
        assert_eq!(r1.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // A retry under the same key must execute again, not replay 500.
        let mk_ok = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::CREATED, b"ok".to_vec())
        };
        let r2 = run_idempotent(&s, Some("retry-me"), b"{}", mk_ok).await;
        assert_eq!(r2.status(), StatusCode::CREATED);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
