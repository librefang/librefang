//! SQLite-backed passkey (WebAuthn/FIDO2) credential store (#5981).
//!
//! Persists the registered platform/roaming authenticators that back the
//! dashboard "Sign in with passkey" flow. The API layer
//! (`librefang-api::passkey`) owns the WebAuthn ceremonies and the
//! `webauthn-rs` types; this module only holds the persistence shape so the
//! memory crate stays free of a `webauthn-rs` dependency. The serialized
//! `Passkey` is stored opaquely in the `cred` column as a JSON string — the
//! whole credential is round-tripped so the updated `sign_count` can be
//! persisted after every successful assertion.
//!
//! Schema is created by migration v44 (see `migration.rs`). Rows never
//! expire; they live until the operator revokes the credential.

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::time::{SystemTime, UNIX_EPOCH};

/// A single stored passkey credential row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyRecord {
    /// Base64url-encoded credential id — primary key and the handle the
    /// revoke endpoint addresses.
    pub credential_id: String,
    /// Principal this credential authenticates as. Bound to the same
    /// identity the password login produces; carried per-row to stay
    /// forward-compatible with the multi-user `[[users]]` path.
    pub user_name: String,
    /// Opaque serialized `webauthn-rs` `Passkey` (JSON). The API layer
    /// deserializes it; the store treats it as a blob.
    pub cred: String,
    /// Optional human-readable label ("MacBook Touch ID", "YubiKey 5").
    pub label: Option<String>,
    /// Unix timestamp (seconds) when the credential was registered.
    pub created_at: i64,
    /// Unix timestamp (seconds) of the most recent successful assertion,
    /// or `None` if it has never been used to sign in.
    pub last_used_at: Option<i64>,
}

/// Pluggable backend so API-crate unit tests can swap in an in-memory
/// implementation. Production wires [`SqlitePasskeyStore`] against the
/// substrate connection pool.
pub trait PasskeyStore: Send + Sync {
    /// Insert a freshly registered credential. Errors on a duplicate
    /// `credential_id` (the authenticator should never re-register the same
    /// credential against the same RP).
    fn insert(&self, record: &PasskeyRecord) -> Result<(), PasskeyStoreError>;

    /// Every credential registered for `user_name`, ordered by `created_at`
    /// ascending. Backs both the authentication ceremony (which needs the
    /// full allow-list) and the Settings list view.
    fn list_for_user(&self, user_name: &str) -> Result<Vec<PasskeyRecord>, PasskeyStoreError>;

    /// Fetch a single credential by its base64url id, or `None` if unknown.
    fn get(&self, credential_id: &str) -> Result<Option<PasskeyRecord>, PasskeyStoreError>;

    /// Persist the re-serialized credential (updated sign-count) and stamp
    /// `last_used_at` after a successful assertion. No-op if the id is
    /// unknown.
    fn update_cred(
        &self,
        credential_id: &str,
        cred: &str,
        last_used_at: i64,
    ) -> Result<(), PasskeyStoreError>;

    /// Revoke a credential. Scoped to `user_name` so one principal can never
    /// delete another's credential. Returns `true` if a row was removed.
    fn delete(&self, credential_id: &str, user_name: &str) -> Result<bool, PasskeyStoreError>;

    /// Number of credentials registered for `user_name`.
    fn count_for_user(&self, user_name: &str) -> Result<u64, PasskeyStoreError>;
}

/// Errors surfaced from the store.
#[derive(Debug)]
pub enum PasskeyStoreError {
    Sqlite(rusqlite::Error),
    Pool(r2d2::Error),
}

impl std::fmt::Display for PasskeyStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasskeyStoreError::Sqlite(e) => write!(f, "sqlite: {}", e),
            PasskeyStoreError::Pool(e) => write!(f, "pool: {}", e),
        }
    }
}

impl std::error::Error for PasskeyStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PasskeyStoreError::Sqlite(e) => Some(e),
            PasskeyStoreError::Pool(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for PasskeyStoreError {
    fn from(e: rusqlite::Error) -> Self {
        PasskeyStoreError::Sqlite(e)
    }
}

impl From<r2d2::Error> for PasskeyStoreError {
    fn from(e: r2d2::Error) -> Self {
        PasskeyStoreError::Pool(e)
    }
}

/// SQLite-backed passkey store reusing the substrate connection pool.
///
/// Sharing the substrate pool (handed out via `MemorySubstrate::pool()`)
/// keeps every persisted byte under one WAL pool — no separate file, no
/// second open call. Mirrors [`crate::idempotency::SqliteIdempotencyStore`].
#[derive(Clone)]
pub struct SqlitePasskeyStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqlitePasskeyStore {
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

/// Increment the shared pool-exhaustion counter so operators can see
/// `pool.get()` failures before they cause user-visible request errors.
fn record_pool_failure(op: &'static str) {
    metrics::counter!(
        "librefang_memory_pool_get_failed_total",
        "store" => "passkey",
        "op" => op,
    )
    .increment(1);
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<PasskeyRecord> {
    Ok(PasskeyRecord {
        credential_id: row.get(0)?,
        user_name: row.get(1)?,
        cred: row.get(2)?,
        label: row.get(3)?,
        created_at: row.get(4)?,
        last_used_at: row.get(5)?,
    })
}

impl PasskeyStore for SqlitePasskeyStore {
    fn insert(&self, record: &PasskeyRecord) -> Result<(), PasskeyStoreError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("insert"))?;
        conn.execute(
            "INSERT INTO webauthn_credentials \
             (credential_id, user_name, cred, label, created_at, last_used_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                record.credential_id,
                record.user_name,
                record.cred,
                record.label,
                record.created_at,
                record.last_used_at,
            ],
        )?;
        Ok(())
    }

    fn list_for_user(&self, user_name: &str) -> Result<Vec<PasskeyRecord>, PasskeyStoreError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("list_for_user"))?;
        let mut stmt = conn.prepare(
            "SELECT credential_id, user_name, cred, label, created_at, last_used_at \
             FROM webauthn_credentials WHERE user_name = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![user_name], row_to_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn get(&self, credential_id: &str) -> Result<Option<PasskeyRecord>, PasskeyStoreError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("get"))?;
        let mut stmt = conn.prepare(
            "SELECT credential_id, user_name, cred, label, created_at, last_used_at \
             FROM webauthn_credentials WHERE credential_id = ?1",
        )?;
        let row = match stmt.query_row(rusqlite::params![credential_id], row_to_record) {
            Ok(record) => Some(record),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(PasskeyStoreError::Sqlite(e)),
        };
        Ok(row)
    }

    fn update_cred(
        &self,
        credential_id: &str,
        cred: &str,
        last_used_at: i64,
    ) -> Result<(), PasskeyStoreError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("update_cred"))?;
        conn.execute(
            "UPDATE webauthn_credentials SET cred = ?2, last_used_at = ?3 \
             WHERE credential_id = ?1",
            rusqlite::params![credential_id, cred, last_used_at],
        )?;
        Ok(())
    }

    fn delete(&self, credential_id: &str, user_name: &str) -> Result<bool, PasskeyStoreError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("delete"))?;
        let affected = conn.execute(
            "DELETE FROM webauthn_credentials WHERE credential_id = ?1 AND user_name = ?2",
            rusqlite::params![credential_id, user_name],
        )?;
        Ok(affected > 0)
    }

    fn count_for_user(&self, user_name: &str) -> Result<u64, PasskeyStoreError> {
        let conn = self
            .pool
            .get()
            .inspect_err(|_| record_pool_failure("count_for_user"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM webauthn_credentials WHERE user_name = ?1",
            rusqlite::params![user_name],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }
}

/// Convenience for the registration path: build a record stamped `created_at
/// = now`, `last_used_at = None`.
pub fn new_record(
    credential_id: String,
    user_name: String,
    cred: String,
    label: Option<String>,
) -> PasskeyRecord {
    PasskeyRecord {
        credential_id,
        user_name,
        cred,
        label,
        created_at: now_unix(),
        last_used_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn make_store() -> SqlitePasskeyStore {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        SqlitePasskeyStore::new(pool)
    }

    fn rec(id: &str, user: &str) -> PasskeyRecord {
        new_record(id.to_string(), user.to_string(), "{}".to_string(), None)
    }

    #[test]
    fn insert_and_list_round_trip() {
        let s = make_store();
        s.insert(&rec("cred-a", "admin")).unwrap();
        s.insert(&rec("cred-b", "admin")).unwrap();
        let list = s.list_for_user("admin").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].credential_id, "cred-a");
        assert_eq!(list[1].credential_id, "cred-b");
        assert_eq!(s.count_for_user("admin").unwrap(), 2);
    }

    #[test]
    fn list_is_scoped_per_user() {
        let s = make_store();
        s.insert(&rec("cred-a", "admin")).unwrap();
        s.insert(&rec("cred-b", "other")).unwrap();
        assert_eq!(s.list_for_user("admin").unwrap().len(), 1);
        assert_eq!(s.list_for_user("other").unwrap().len(), 1);
        assert_eq!(s.count_for_user("nobody").unwrap(), 0);
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let s = make_store();
        assert!(s.get("missing").unwrap().is_none());
        s.insert(&rec("cred-a", "admin")).unwrap();
        assert_eq!(s.get("cred-a").unwrap().unwrap().user_name, "admin");
    }

    #[test]
    fn duplicate_insert_is_rejected() {
        let s = make_store();
        s.insert(&rec("cred-a", "admin")).unwrap();
        assert!(s.insert(&rec("cred-a", "admin")).is_err());
    }

    #[test]
    fn update_cred_persists_sign_count_and_last_used() {
        let s = make_store();
        s.insert(&rec("cred-a", "admin")).unwrap();
        s.update_cred("cred-a", "{\"sign_count\":5}", 1234).unwrap();
        let got = s.get("cred-a").unwrap().unwrap();
        assert_eq!(got.cred, "{\"sign_count\":5}");
        assert_eq!(got.last_used_at, Some(1234));
        // Updating an unknown id is a silent no-op.
        s.update_cred("missing", "{}", 1).unwrap();
    }

    #[test]
    fn delete_is_scoped_to_user() {
        let s = make_store();
        s.insert(&rec("cred-a", "admin")).unwrap();
        // Wrong user can't revoke.
        assert!(!s.delete("cred-a", "intruder").unwrap());
        assert_eq!(s.count_for_user("admin").unwrap(), 1);
        // Correct user can.
        assert!(s.delete("cred-a", "admin").unwrap());
        assert_eq!(s.count_for_user("admin").unwrap(), 0);
        // Deleting again reports no row removed.
        assert!(!s.delete("cred-a", "admin").unwrap());
    }
}
