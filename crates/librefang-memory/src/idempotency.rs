//! SQLite-backed Idempotency-Key cache shared by the API layer (#3637).
//!
//! The HTTP middleware that owns Idempotency-Key semantics lives in
//! `librefang-api::idempotency`; this module just holds the persistence
//! shape so the API crate doesn't need to depend on `rusqlite`
//! directly. Schema is created by migration v34 (see `migration.rs`).
//!
//! A status-zero row reserves a key while its handler runs, and a successful handler atomically completes that row with its replayable response.
//! Completed responses remain replayable for 24 hours after completion.
//! Pending reservations do not expire automatically: cancellation and non-success responses release them, while an abnormal process exit fails closed until an operator explicitly removes the orphaned reservation.

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

/// 24-hour replay window per #3637.
pub const TTL_SECONDS: i64 = 24 * 60 * 60;

/// Avoid issuing a table-wide DELETE on every idempotent request.
const PRUNE_INTERVAL_SECONDS: i64 = 60;

/// Cached HTTP response replayed verbatim on subsequent matching
/// requests. Status is stored as `u16` to keep the row schema flat;
/// the API layer rebuilds an `axum::Response` from these bytes.
#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Persisted idempotency record.
#[derive(Debug, Clone)]
pub struct StoredRecord {
    pub body_hash: String,
    pub response: CachedResponse,
}

/// Result of atomically reserving an idempotency key before a handler runs.
#[derive(Debug)]
pub enum Reservation {
    Acquired { token: ReservationToken },
    Existing(StoredRecord),
}

/// Unforgeable ownership token for one pending reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationToken(String);

impl ReservationToken {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl Default for ReservationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Sentinel stored while the first request is still executing.
pub const PENDING_STATUS: u16 = 0;

/// Pluggable backend so unit tests in the API crate can swap in an
/// in-memory implementation. Production wires
/// [`SqliteIdempotencyStore`] against the substrate connection.
pub trait IdempotencyStore: Send + Sync {
    /// Look up an existing record by key. Expired completed rows are deleted;
    /// pending reservations fail closed and are never expired automatically.
    fn lookup(&self, key: &str) -> Result<Option<StoredRecord>, IdempotencyError>;

    /// Atomically reserve a key before executing its handler.
    fn reserve(&self, key: &str, body_hash: &str) -> Result<Reservation, IdempotencyError>;

    /// Replace an owned pending reservation with its replayable response.
    fn complete(
        &self,
        key: &str,
        body_hash: &str,
        token: &ReservationToken,
        response: &CachedResponse,
    ) -> Result<(), IdempotencyError>;

    /// Remove an owned pending reservation after a non-success or cancellation.
    fn release(
        &self,
        key: &str,
        body_hash: &str,
        token: &ReservationToken,
    ) -> Result<(), IdempotencyError>;

    /// Delete expired completed responses. Pending reservations are retained
    /// so a long-running handler cannot lose ownership to a retry.
    fn prune_expired(&self) -> Result<(), IdempotencyError>;
}

/// Errors surfaced from the store.
#[derive(Debug)]
pub enum IdempotencyError {
    Sqlite(rusqlite::Error),
    Pool(r2d2::Error),
    Invariant(String),
}

impl std::fmt::Display for IdempotencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdempotencyError::Sqlite(e) => write!(f, "sqlite: {}", e),
            IdempotencyError::Pool(e) => write!(f, "pool: {}", e),
            IdempotencyError::Invariant(e) => write!(f, "invariant: {e}"),
        }
    }
}

impl std::error::Error for IdempotencyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IdempotencyError::Sqlite(e) => Some(e),
            IdempotencyError::Pool(e) => Some(e),
            IdempotencyError::Invariant(_) => None,
        }
    }
}

impl From<rusqlite::Error> for IdempotencyError {
    fn from(e: rusqlite::Error) -> Self {
        IdempotencyError::Sqlite(e)
    }
}

impl From<r2d2::Error> for IdempotencyError {
    fn from(e: r2d2::Error) -> Self {
        IdempotencyError::Pool(e)
    }
}

/// SQLite-backed idempotency store reusing the substrate connection.
///
/// Sharing the substrate connection pool (handed out via
/// `MemorySubstrate::pool()`) keeps every persisted byte under one
/// WAL pool — no separate file, no second open call.
#[derive(Clone)]
pub struct SqliteIdempotencyStore {
    pool: Pool<SqliteConnectionManager>,
    last_pruned_at: Arc<AtomicI64>,
}

impl SqliteIdempotencyStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            last_pruned_at: Arc::new(AtomicI64::new(0)),
        }
    }
}

fn unix_seconds(time: SystemTime) -> Result<i64, IdempotencyError> {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            IdempotencyError::Invariant(format!("system clock precedes Unix epoch: {error}"))
        })?
        .as_secs();
    i64::try_from(seconds).map_err(|_| {
        IdempotencyError::Invariant("system clock exceeds SQLite timestamp range".to_string())
    })
}

fn now_unix() -> Result<i64, IdempotencyError> {
    unix_seconds(SystemTime::now())
}

fn decode_record(
    body_hash: String,
    status: i64,
    body: Vec<u8>,
) -> Result<StoredRecord, IdempotencyError> {
    let status = u16::try_from(status).map_err(|_| {
        IdempotencyError::Invariant(format!(
            "idempotency response status {status} is outside the u16 range"
        ))
    })?;
    Ok(StoredRecord {
        body_hash,
        response: CachedResponse { status, body },
    })
}

/// Increment the shared pool-exhaustion counter so operators can see
/// `pool.get()` failures before they cause user-visible request errors.
fn record_pool_failure(op: &'static str) {
    metrics::counter!(
        "librefang_memory_pool_get_failed_total",
        "store" => "idempotency",
        "op" => op,
    )
    .increment(1);
}

impl IdempotencyStore for SqliteIdempotencyStore {
    fn lookup(&self, key: &str) -> Result<Option<StoredRecord>, IdempotencyError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("lookup"))?;
        let now = now_unix()?;
        // Replay TTL applies only after successful completion.
        // A pending row remains a conflict even if its legacy expires_at value is in the past.
        conn.execute(
            "DELETE FROM idempotency_keys \
             WHERE key = ?1 AND response_status != ?2 AND expires_at <= ?3",
            rusqlite::params![key, PENDING_STATUS as i64, now],
        )?;
        let mut stmt = conn.prepare(
            "SELECT body_hash, response_status, response_body \
             FROM idempotency_keys WHERE key = ?1",
        )?;
        let row: Option<(String, i64, Vec<u8>)> = stmt
            .query_row(rusqlite::params![key], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        row.map(|(body_hash, status, body)| decode_record(body_hash, status, body))
            .transpose()
    }

    fn reserve(&self, key: &str, body_hash: &str) -> Result<Reservation, IdempotencyError> {
        let mut conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("reserve"))?;
        let now = now_unix()?;
        let token = ReservationToken::new();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM idempotency_keys \
             WHERE key = ?1 AND response_status != ?2 AND expires_at <= ?3",
            rusqlite::params![key, PENDING_STATUS as i64, now],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO idempotency_keys \
             (key, body_hash, response_status, response_body, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                key,
                body_hash,
                PENDING_STATUS as i64,
                token.as_bytes(),
                now,
                i64::MAX
            ],
        )?;
        let reservation = if inserted == 1 {
            Reservation::Acquired { token }
        } else {
            let (stored_hash, status, body): (String, i64, Vec<u8>) = tx.query_row(
                "SELECT body_hash, response_status, response_body \
                 FROM idempotency_keys WHERE key = ?1",
                rusqlite::params![key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            Reservation::Existing(decode_record(stored_hash, status, body)?)
        };
        tx.commit()?;
        Ok(reservation)
    }

    fn complete(
        &self,
        key: &str,
        body_hash: &str,
        token: &ReservationToken,
        response: &CachedResponse,
    ) -> Result<(), IdempotencyError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("complete"))?;
        let now = now_unix()?;
        let expires = now + TTL_SECONDS;
        let updated = conn.execute(
            "UPDATE idempotency_keys \
             SET response_status = ?3, response_body = ?4, expires_at = ?5 \
             WHERE key = ?1 AND body_hash = ?2 AND response_status = 0 \
               AND response_body = ?6",
            rusqlite::params![
                key,
                body_hash,
                response.status as i64,
                response.body,
                expires,
                token.as_bytes()
            ],
        )?;
        if updated != 1 {
            return Err(IdempotencyError::Invariant(format!(
                "pending reservation for key {key:?} was lost before completion"
            )));
        }
        Ok(())
    }

    fn release(
        &self,
        key: &str,
        body_hash: &str,
        token: &ReservationToken,
    ) -> Result<(), IdempotencyError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("release"))?;
        conn.execute(
            "DELETE FROM idempotency_keys \
             WHERE key = ?1 AND body_hash = ?2 AND response_status = 0 \
               AND response_body = ?3",
            rusqlite::params![key, body_hash, token.as_bytes()],
        )?;
        Ok(())
    }

    fn prune_expired(&self) -> Result<(), IdempotencyError> {
        let now = now_unix()?;
        let last = self.last_pruned_at.load(Ordering::Relaxed);
        if now >= last && now.saturating_sub(last) < PRUNE_INTERVAL_SECONDS {
            return Ok(());
        }
        if self
            .last_pruned_at
            .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return Ok(());
        }
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("prune_expired"))
            .inspect_err(|_| {
                let _ = self.last_pruned_at.compare_exchange(
                    now,
                    last,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
            })?;
        let result = conn.execute(
            "DELETE FROM idempotency_keys \
             WHERE response_status != ?1 AND expires_at <= ?2",
            rusqlite::params![PENDING_STATUS as i64, now],
        );
        if result.is_err() {
            let _ = self.last_pruned_at.compare_exchange(
                now,
                last,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }
        result?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn make_store() -> SqliteIdempotencyStore {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        SqliteIdempotencyStore::new(pool)
    }

    #[test]
    fn reserve_complete_and_lookup_round_trip() {
        let s = make_store();
        let resp = CachedResponse {
            status: 200,
            body: b"{\"ok\":true}".to_vec(),
        };
        let token = match s.reserve("k1", "h1").unwrap() {
            Reservation::Acquired { token } => token,
            Reservation::Existing(_) => panic!("first caller must acquire"),
        };
        s.complete("k1", "h1", &token, &resp).unwrap();
        let got = s.lookup("k1").unwrap().expect("hit");
        assert_eq!(got.body_hash, "h1");
        assert_eq!(got.response.status, 200);
        assert_eq!(got.response.body, b"{\"ok\":true}");
    }

    #[test]
    fn reservation_is_first_writer_wins() {
        let s = make_store();
        let r1 = CachedResponse {
            status: 200,
            body: b"first".to_vec(),
        };
        let token = match s.reserve("k", "h").unwrap() {
            Reservation::Acquired { token } => token,
            Reservation::Existing(_) => panic!("first caller must acquire"),
        };
        assert!(matches!(
            s.reserve("k", "h").unwrap(),
            Reservation::Existing(StoredRecord {
                response: CachedResponse {
                    status: PENDING_STATUS,
                    ..
                },
                ..
            })
        ));
        s.complete("k", "h", &token, &r1).unwrap();
        let got = s.lookup("k").unwrap().expect("hit");
        assert_eq!(got.response.body, b"first");
    }

    #[test]
    fn expired_row_is_treated_as_miss() {
        let s = make_store();
        // Insert an already-expired row directly.
        {
            let conn = s.pool.get().expect("pool");
            conn.execute(
                "INSERT INTO idempotency_keys \
                 (key, body_hash, response_status, response_body, created_at, expires_at) \
                 VALUES ('old', 'h', 200, x'00', ?1, ?2)",
                rusqlite::params![now_unix().unwrap() - 100_000, now_unix().unwrap() - 1],
            )
            .unwrap();
        }
        assert!(s.lookup("old").unwrap().is_none());
    }

    #[test]
    fn concurrent_reservations_have_exactly_one_owner() {
        let temp = tempfile::tempdir().unwrap();
        let pool = Pool::builder()
            .max_size(2)
            .build(
                SqliteConnectionManager::file(temp.path().join("idempotency.db"))
                    .with_init(|c| c.execute_batch(crate::substrate::DEFAULT_CONNECTION_PRAGMAS)),
            )
            .unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        let store = SqliteIdempotencyStore::new(pool);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let workers: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.reserve("race", "same-body").unwrap()
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();

        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Reservation::Acquired { .. }))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(
                    result,
                    Reservation::Existing(StoredRecord {
                        body_hash,
                        response: CachedResponse {
                            status: PENDING_STATUS,
                            ..
                        },
                    }) if body_hash == "same-body"
                ))
                .count(),
            1
        );
    }

    #[test]
    fn expired_pending_reservation_is_not_reclaimed() {
        let store = make_store();
        match store.reserve("pending", "same-body").unwrap() {
            Reservation::Acquired { .. } => {}
            Reservation::Existing(_) => panic!("first caller must acquire"),
        }
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE idempotency_keys SET expires_at = ?2 WHERE key = ?1",
                rusqlite::params!["pending", now_unix().unwrap() - 1],
            )
            .unwrap();
        }
        store.prune_expired().unwrap();
        assert_eq!(
            store
                .lookup("pending")
                .unwrap()
                .expect("pending row must survive lookup and pruning")
                .response
                .status,
            PENDING_STATUS
        );
        assert!(matches!(
            store.reserve("pending", "same-body").unwrap(),
            Reservation::Existing(StoredRecord {
                response: CachedResponse {
                    status: PENDING_STATUS,
                    ..
                },
                ..
            })
        ));
    }

    #[test]
    fn superseded_owner_cannot_complete_or_release_replacement() {
        let store = make_store();
        let first_token = match store.reserve("replaced", "same-body").unwrap() {
            Reservation::Acquired { token } => token,
            Reservation::Existing(_) => panic!("first caller must acquire"),
        };
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "DELETE FROM idempotency_keys WHERE key = ?1",
                rusqlite::params!["replaced"],
            )
            .unwrap();
        }
        let replacement_token = match store.reserve("replaced", "same-body").unwrap() {
            Reservation::Acquired { token } => token,
            Reservation::Existing(_) => panic!("replacement caller must acquire"),
        };
        assert_ne!(first_token, replacement_token);

        let response = CachedResponse {
            status: 201,
            body: b"replacement".to_vec(),
        };
        assert!(store
            .complete("replaced", "same-body", &first_token, &response)
            .is_err());
        store
            .release("replaced", "same-body", &first_token)
            .unwrap();
        store
            .complete("replaced", "same-body", &replacement_token, &response)
            .expect("old owner must leave replacement reservation intact");
    }

    #[test]
    fn pre_epoch_time_is_rejected() {
        let before_epoch = UNIX_EPOCH - std::time::Duration::from_secs(1);
        assert!(unix_seconds(before_epoch).is_err());
    }

    #[test]
    fn corrupt_out_of_range_status_is_rejected() {
        let store = make_store();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO idempotency_keys \
             (key, body_hash, response_status, response_body, created_at, expires_at) \
             VALUES ('corrupt', 'h', 65736, x'00', ?1, ?2)",
            rusqlite::params![now_unix().unwrap(), now_unix().unwrap() + TTL_SECONDS],
        )
        .unwrap();
        drop(conn);

        assert!(store.lookup("corrupt").is_err());
    }

    #[test]
    fn reserve_rejects_corrupt_out_of_range_status() {
        let store = make_store();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO idempotency_keys \
             (key, body_hash, response_status, response_body, created_at, expires_at) \
             VALUES ('corrupt-reserve', 'h', 65736, x'00', ?1, ?2)",
            rusqlite::params![now_unix().unwrap(), now_unix().unwrap() + TTL_SECONDS],
        )
        .unwrap();
        drop(conn);

        assert!(store.reserve("corrupt-reserve", "h").is_err());
    }

    #[test]
    fn repeated_prune_calls_are_rate_limited() {
        let store = make_store();
        store.prune_expired().unwrap();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO idempotency_keys \
             (key, body_hash, response_status, response_body, created_at, expires_at) \
             VALUES ('recent-prune', 'h', 200, x'00', ?1, ?2)",
            rusqlite::params![now_unix().unwrap() - 2, now_unix().unwrap() - 1],
        )
        .unwrap();
        drop(conn);

        store.prune_expired().unwrap();
        let conn = store.pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM idempotency_keys WHERE key = 'recent-prune'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "second prune inside the interval must be a no-op");
    }

    #[test]
    fn clock_rollback_reestablishes_prune_baseline() {
        let store = make_store();
        let now = now_unix().unwrap();
        store.last_pruned_at.store(now + 3600, Ordering::Release);
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO idempotency_keys \
             (key, body_hash, response_status, response_body, created_at, expires_at) \
             VALUES ('rollback-prune', 'h', 200, x'00', ?1, ?2)",
            rusqlite::params![now - 2, now - 1],
        )
        .unwrap();
        drop(conn);

        store.prune_expired().unwrap();
        let conn = store.pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM idempotency_keys WHERE key = 'rollback-prune'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "rollback must allow one prune winner");
    }
}
