//! SQLite-backed Idempotency-Key cache shared by the API layer (#3637).
//!
//! The HTTP middleware that owns Idempotency-Key semantics lives in
//! `librefang-api::idempotency`; this module just holds the persistence
//! shape so the API crate doesn't need to depend on `rusqlite`
//! directly. Schema is created by migration v34 (see `migration.rs`).
//!
//! Records expire 24h after creation. Lookup deletes expired rows
//! opportunistically so the table self-trims without a background job.

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::time::{SystemTime, UNIX_EPOCH};

/// 24-hour replay window per #3637.
pub const TTL_SECONDS: i64 = 24 * 60 * 60;

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

/// Pluggable backend so unit tests in the API crate can swap in an
/// in-memory implementation. Production wires
/// [`SqliteIdempotencyStore`] against the substrate connection.
pub trait IdempotencyStore: Send + Sync {
    /// Look up an existing record by key. Expired rows are deleted in
    /// place and reported as `Ok(None)` so the caller treats them as a
    /// fresh miss.
    fn lookup(&self, key: &str) -> Result<Option<StoredRecord>, IdempotencyError>;

    /// Persist a fresh record. First-writer-wins via `INSERT OR
    /// IGNORE`: a concurrent insert under the same key is a silent
    /// no-op (the canonical reply is whichever landed first).
    fn put(
        &self,
        key: &str,
        body_hash: &str,
        response: &CachedResponse,
    ) -> Result<(), IdempotencyError>;

    /// Delete every expired row. Called opportunistically by the
    /// middleware so the table self-trims.
    fn prune_expired(&self) -> Result<(), IdempotencyError>;
}

/// Errors surfaced from the store.
#[derive(Debug)]
pub enum IdempotencyError {
    Sqlite(rusqlite::Error),
    Pool(r2d2::Error),
}

impl std::fmt::Display for IdempotencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdempotencyError::Sqlite(e) => write!(f, "sqlite: {}", e),
            IdempotencyError::Pool(e) => write!(f, "pool: {}", e),
        }
    }
}

impl std::error::Error for IdempotencyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IdempotencyError::Sqlite(e) => Some(e),
            IdempotencyError::Pool(e) => Some(e),
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
}

impl SqliteIdempotencyStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl IdempotencyStore for SqliteIdempotencyStore {
    fn lookup(&self, key: &str) -> Result<Option<StoredRecord>, IdempotencyError> {
        let conn = self.pool.get()?;
        let now = now_unix();
        // Drop the row if it's expired so the lookup behaves like a
        // fresh miss; the write path will then re-INSERT cleanly.
        conn.execute(
            "DELETE FROM idempotency_keys WHERE key = ?1 AND expires_at <= ?2",
            rusqlite::params![key, now],
        )?;
        let mut stmt = conn.prepare(
            "SELECT body_hash, response_status, response_body \
             FROM idempotency_keys WHERE key = ?1",
        )?;
        let row = stmt
            .query_row(rusqlite::params![key], |row| {
                let body_hash: String = row.get(0)?;
                let status: i64 = row.get(1)?;
                let body: Vec<u8> = row.get(2)?;
                Ok(StoredRecord {
                    body_hash,
                    response: CachedResponse {
                        status: status as u16,
                        body,
                    },
                })
            })
            .ok();
        Ok(row)
    }

    fn put(
        &self,
        key: &str,
        body_hash: &str,
        response: &CachedResponse,
    ) -> Result<(), IdempotencyError> {
        let conn = self.pool.get()?;
        let now = now_unix();
        let expires = now + TTL_SECONDS;
        conn.execute(
            "INSERT OR IGNORE INTO idempotency_keys \
             (key, body_hash, response_status, response_body, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                key,
                body_hash,
                response.status as i64,
                response.body,
                now,
                expires
            ],
        )?;
        Ok(())
    }

    fn prune_expired(&self) -> Result<(), IdempotencyError> {
        let conn = self.pool.get()?;
        let now = now_unix();
        conn.execute(
            "DELETE FROM idempotency_keys WHERE expires_at <= ?1",
            rusqlite::params![now],
        )?;
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
    fn put_and_lookup_round_trip() {
        let s = make_store();
        let resp = CachedResponse {
            status: 200,
            body: b"{\"ok\":true}".to_vec(),
        };
        s.put("k1", "h1", &resp).unwrap();
        let got = s.lookup("k1").unwrap().expect("hit");
        assert_eq!(got.body_hash, "h1");
        assert_eq!(got.response.status, 200);
        assert_eq!(got.response.body, b"{\"ok\":true}");
    }

    #[test]
    fn first_writer_wins() {
        let s = make_store();
        let r1 = CachedResponse {
            status: 200,
            body: b"first".to_vec(),
        };
        let r2 = CachedResponse {
            status: 200,
            body: b"second".to_vec(),
        };
        s.put("k", "h", &r1).unwrap();
        s.put("k", "h", &r2).unwrap();
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
                rusqlite::params![now_unix() - 100_000, now_unix() - 1],
            )
            .unwrap();
        }
        assert!(s.lookup("old").unwrap().is_none());
    }
}
