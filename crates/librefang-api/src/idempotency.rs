//! Idempotency-Key replay middleware for state-creating POSTs (#3637).
//!
//! Opt-in: callers signal "this request is replay-safe" by sending an
//! `Idempotency-Key: <opaque-string>` header. When set, the handler runs
//! through [`run_idempotent`], which:
//!
//! 1. Atomically reserves `(key)` in the persistent store.
//! 2. **Reservation acquired**: executes the inner handler, then completes the reservation with the successful 2xx response for 24 hours.
//!    Non-2xx responses are not cached so a transient failure (rate limit, downstream blip) does not poison the slot — clients can retry the same key and get a real attempt.
//! 3. **Concurrent reservation, same body**: returns 409 without running the handler.
//! 4. **Cache hit, same body**: replays the cached `(status, body)` without re-executing the handler.
//! 5. **Cache hit, different body**: returns 409 Conflict.
//!    The `Idempotency-Key` is the operator-supplied dedup token and a different payload under the same key is a programming error (e.g. UI accidentally reuses an old key after editing the form).
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
use librefang_memory::idempotency::{
    CachedResponse, IdempotencyStore, Reservation, ReservationToken, PENDING_STATUS,
};
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

/// 409 response returned while the first request for this key is still running.
pub fn request_in_progress_response() -> Response {
    let payload = serde_json::json!({
        "error": "A request with this Idempotency-Key is already in progress",
        "code": "idempotency_key_in_use",
        "type": "idempotency_key_in_use",
    });
    (StatusCode::CONFLICT, Json(payload)).into_response()
}

fn store_unavailable_response() -> Response {
    let payload = serde_json::json!({
        "error": "Idempotency storage is unavailable",
        "code": "idempotency_store_unavailable",
        "type": "idempotency_store_unavailable",
    });
    (StatusCode::SERVICE_UNAVAILABLE, Json(payload)).into_response()
}

struct ReservationGuard<'a> {
    store: &'a dyn IdempotencyStore,
    key: &'a str,
    body_hash: &'a str,
    token: ReservationToken,
    armed: bool,
}

impl<'a> ReservationGuard<'a> {
    fn new(
        store: &'a dyn IdempotencyStore,
        key: &'a str,
        body_hash: &'a str,
        token: ReservationToken,
    ) -> Self {
        Self {
            store,
            key,
            body_hash,
            token,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ReservationGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.store.release(self.key, self.body_hash, &self.token) {
                tracing::error!(
                    key = self.key,
                    %error,
                    "failed to release abandoned idempotency reservation"
                );
            }
        }
    }
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

    // Reserve before starting the handler.
    // The SQLite implementation performs expiration cleanup, insert, and existing-row lookup under one immediate transaction, so same-key requests cannot both observe a miss.
    let reservation = match store.reserve(key, &body_hash) {
        Ok(reservation) => reservation,
        Err(e) => {
            tracing::error!(key, error = %e, "idempotency reservation failed");
            return store_unavailable_response();
        }
    };

    let token = match reservation {
        Reservation::Existing(existing) => {
            if existing.body_hash != body_hash {
                return body_conflict_response();
            }
            if existing.response.status == PENDING_STATUS {
                return request_in_progress_response();
            }
            let Ok(status) = StatusCode::from_u16(existing.response.status) else {
                tracing::error!(
                    key,
                    status = existing.response.status,
                    "idempotency cache contains an invalid HTTP status"
                );
                return store_unavailable_response();
            };
            return build_response(status, existing.response.body);
        }
        Reservation::Acquired { token } => token,
    };

    let mut guard = ReservationGuard::new(store, key, &body_hash, token);
    let (status, body) = f().await;
    if status.is_success() {
        let cached = CachedResponse {
            status: status.as_u16(),
            body: body.clone(),
        };
        if let Err(e) = store.complete(key, &body_hash, &guard.token, &cached) {
            // Keep the pending row instead of making the key reusable after the side effect already happened.
            // This remains fail-closed until an operator has verified the side effect and removes the orphan.
            guard.disarm();
            tracing::error!(key, error = %e, "idempotency completion failed");
            return store_unavailable_response();
        }
        guard.disarm();
    }
    // A non-2xx response leaves the guard armed, releasing the reservation so the key remains retriable.
    // Cancellation and panic take the same Drop path.
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
        fail_reserve: std::sync::atomic::AtomicBool,
        fail_complete: std::sync::atomic::AtomicBool,
    }
    impl IdempotencyStore for MemStore {
        fn lookup(&self, key: &str) -> Result<Option<StoredRecord>, IdempotencyError> {
            Ok(self.rows.lock().unwrap().get(key).cloned())
        }
        fn reserve(&self, key: &str, body_hash: &str) -> Result<Reservation, IdempotencyError> {
            if self.fail_reserve.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(IdempotencyError::Invariant(
                    "injected reserve failure".to_string(),
                ));
            }
            let mut rows = self.rows.lock().unwrap();
            if let Some(existing) = rows.get(key) {
                return Ok(Reservation::Existing(existing.clone()));
            }
            let token = ReservationToken::new();
            rows.insert(
                key.to_string(),
                StoredRecord {
                    body_hash: body_hash.to_string(),
                    response: CachedResponse {
                        status: PENDING_STATUS,
                        body: token.as_bytes().to_vec(),
                    },
                },
            );
            Ok(Reservation::Acquired { token })
        }
        fn complete(
            &self,
            key: &str,
            body_hash: &str,
            token: &ReservationToken,
            response: &CachedResponse,
        ) -> Result<(), IdempotencyError> {
            if self.fail_complete.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(IdempotencyError::Invariant(
                    "injected complete failure".to_string(),
                ));
            }
            let mut rows = self.rows.lock().unwrap();
            let row = rows.get_mut(key).ok_or_else(|| {
                IdempotencyError::Invariant("test reservation missing".to_string())
            })?;
            if row.body_hash != body_hash
                || row.response.status != PENDING_STATUS
                || row.response.body != token.as_bytes()
            {
                return Err(IdempotencyError::Invariant(
                    "test reservation ownership mismatch".to_string(),
                ));
            }
            *row = StoredRecord {
                body_hash: body_hash.to_string(),
                response: response.clone(),
            };
            Ok(())
        }
        fn release(
            &self,
            key: &str,
            body_hash: &str,
            token: &ReservationToken,
        ) -> Result<(), IdempotencyError> {
            let mut rows = self.rows.lock().unwrap();
            if rows.get(key).is_some_and(|row| {
                row.body_hash == body_hash
                    && row.response.status == PENDING_STATUS
                    && row.response.body == token.as_bytes()
            }) {
                rows.remove(key);
            }
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
    async fn concurrent_same_key_executes_handler_once() {
        let store = MemStore::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();

        let first = run_idempotent(&store, Some("in-flight"), b"{}", || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            (StatusCode::CREATED, b"first".to_vec())
        });
        let second = async {
            entered_rx.await.unwrap();
            let response = run_idempotent(&store, Some("in-flight"), b"{}", || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (StatusCode::CREATED, b"second".to_vec())
            })
            .await;
            release_tx.send(()).unwrap();
            response
        };

        let (first_response, second_response) =
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                tokio::join!(first, second)
            })
            .await
            .expect("condition-synchronized requests must complete");

        assert_eq!(first_response.status(), StatusCode::CREATED);
        assert_eq!(second_response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(second_response.into_body(), 4096)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["code"], "idempotency_key_in_use");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_handler_releases_pending_reservation() {
        let store = std::sync::Arc::new(MemStore::default());
        let task_store = store.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            run_idempotent(&*task_store, Some("cancelled"), b"{}", || async {
                entered_tx.send(()).unwrap();
                std::future::pending::<(StatusCode, Vec<u8>)>().await
            })
            .await
        });

        entered_rx.await.unwrap();
        assert_eq!(
            store
                .lookup("cancelled")
                .unwrap()
                .expect("pending reservation")
                .response
                .status,
            PENDING_STATUS
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(store.lookup("cancelled").unwrap().is_none());

        let retry = run_idempotent(&*store, Some("cancelled"), b"{}", || async {
            (StatusCode::CREATED, b"retry".to_vec())
        })
        .await;
        assert_eq!(retry.status(), StatusCode::CREATED);
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

    #[tokio::test]
    async fn reserve_failure_does_not_run_handler() {
        let store = MemStore::default();
        store
            .fail_reserve
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let calls = std::sync::atomic::AtomicUsize::new(0);

        let response = run_idempotent(&store, Some("reserve-fails"), b"{}", || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::CREATED, b"created".to_vec())
        })
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn complete_failure_fails_closed_without_reexecuting_handler() {
        let store = MemStore::default();
        store
            .fail_complete
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let handler = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::CREATED, b"created".to_vec())
        };

        let first = run_idempotent(&store, Some("complete-fails"), b"{}", handler).await;
        assert_eq!(first.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            store
                .lookup("complete-fails")
                .unwrap()
                .expect("pending row retained")
                .response
                .status,
            PENDING_STATUS
        );

        let retry = run_idempotent(&store, Some("complete-fails"), b"{}", handler).await;
        assert_eq!(retry.status(), StatusCode::CONFLICT);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
