//! SurrealDB-backed [`crate::storage_backends::TotpLockoutBackend`].
//!
//! Persists TOTP lockout rows through the shared
//! [`librefang_storage::SurrealSession`]. The schema lives in migration
//! `004_totp_lockout` (see `librefang-storage::migrations`).
//!
//! ## Design notes
//!
//! - Sender ids may contain channel-specific characters (`":"`, `"/"`)
//!   so we sanitise them into a SurrealDB-safe record id before
//!   round-tripping. The original `sender_id` is still stored verbatim
//!   inside the row so it survives the sanitisation.
//! - `locked_at_unix` stays an integer (Unix-seconds since epoch); we
//!   deliberately do not convert it to a SurrealDB `datetime` so the
//!   schemaful round-trip stays clean (same trade-off as the trace and
//!   audit backends made in Phase 5/6).

use librefang_storage::SurrealSession;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;

use crate::storage_backends::{
    TotpLockoutBackend, TotpLockoutError, TotpLockoutResult, TotpLockoutRow,
};

/// SurrealDB-backed implementation of [`TotpLockoutBackend`].
///
/// Construct via [`SurrealTotpLockoutBackend::open`]; the session must
/// already have its namespace and database selected, and the migrations
/// in [`librefang_storage::migrations`] must have run.
pub struct SurrealTotpLockoutBackend {
    db: Surreal<Any>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockoutRow {
    sender_id: String,
    failures: i64,
    locked_at: Option<i64>,
}

impl SurrealTotpLockoutBackend {
    /// Open the backend against an existing session.
    #[must_use]
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: session.client().clone(),
        }
    }
}

impl TotpLockoutBackend for SurrealTotpLockoutBackend {
    fn load_all(&self) -> TotpLockoutResult<Vec<TotpLockoutRow>> {
        // SurrealDB 3.0's `take::<R>` requires `R: SurrealValue`; we
        // round-trip through `serde_json::Value` (which implements that
        // trait) and then decode into our local `LockoutRow` so we don't
        // have to derive `SurrealValue` on every persisted struct.
        let rows: Vec<serde_json::Value> = block_on(async {
            self.db
                .query("SELECT sender_id, failures, locked_at FROM totp_lockout")
                .await
                .map_err(|e| TotpLockoutError::Backend(e.to_string()))?
                .take(0)
                .map_err(|e| TotpLockoutError::Backend(e.to_string()))
        })?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_value::<LockoutRow>(row) {
                Ok(r) => out.push(TotpLockoutRow {
                    sender_id: r.sender_id,
                    failures: u32::try_from(r.failures).unwrap_or(u32::MAX),
                    locked_at_unix: r.locked_at,
                }),
                Err(e) => {
                    return Err(TotpLockoutError::Backend(format!(
                        "decode totp_lockout row: {e}"
                    )))
                }
            }
        }
        Ok(out)
    }

    fn upsert(&self, row: &TotpLockoutRow) -> TotpLockoutResult<()> {
        let id = sanitise_id(&row.sender_id);
        // SurrealDB 3.0 rejects JSON `null` for fields typed as
        // `option<int>` ("Expected `none | int` but found `NULL`"), so
        // we omit `locked_at` from the payload entirely when the lockout
        // window has not yet started; the schemaful field default
        // `option<int>` then resolves to `NONE`.
        let mut payload = serde_json::Map::new();
        payload.insert(
            "sender_id".into(),
            serde_json::Value::String(row.sender_id.clone()),
        );
        payload.insert(
            "failures".into(),
            serde_json::Value::Number(i64::from(row.failures).into()),
        );
        if let Some(ts) = row.locked_at_unix {
            payload.insert("locked_at".into(), serde_json::Value::Number(ts.into()));
        }
        let payload = serde_json::Value::Object(payload);
        block_on(async {
            let _: Option<serde_json::Value> = self
                .db
                .upsert(("totp_lockout", id.as_str()))
                .content(payload)
                .await
                .map_err(|e| TotpLockoutError::Backend(e.to_string()))?;
            Ok::<(), TotpLockoutError>(())
        })
    }

    fn clear(&self, sender_id: &str) -> TotpLockoutResult<()> {
        let id = sanitise_id(sender_id);
        block_on(async {
            let _: Option<serde_json::Value> = self
                .db
                .delete(("totp_lockout", id.as_str()))
                .await
                .map_err(|e| TotpLockoutError::Backend(e.to_string()))?;
            Ok::<(), TotpLockoutError>(())
        })
    }
}

fn sanitise_id(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn block_on<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build temporary tokio runtime")
            .block_on(fut),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_storage::{StorageBackendKind, StorageConfig, SurrealConnectionPool};
    use tempfile::tempdir;

    async fn open_backend(path: &std::path::Path) -> SurrealTotpLockoutBackend {
        let pool = SurrealConnectionPool::new();
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(path.to_path_buf()),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = pool.open(&cfg).await.expect("open session");
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .expect("migrations");
        SurrealTotpLockoutBackend::open(&session)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn round_trip_lockout_rows() {
        let dir = tempdir().unwrap();
        let backend = open_backend(&dir.path().join("kernel.surreal")).await;

        backend
            .upsert(&TotpLockoutRow {
                sender_id: "slack:U12345".into(),
                failures: 3,
                locked_at_unix: None,
            })
            .expect("upsert pending");
        backend
            .upsert(&TotpLockoutRow {
                sender_id: "slack:U67890".into(),
                failures: 5,
                locked_at_unix: Some(1_700_000_000),
            })
            .expect("upsert locked");

        let rows = backend.load_all().expect("load all");
        assert_eq!(rows.len(), 2);
        let by_sender: std::collections::HashMap<_, _> =
            rows.into_iter().map(|r| (r.sender_id.clone(), r)).collect();

        let pending = by_sender.get("slack:U12345").expect("pending row");
        assert_eq!(pending.failures, 3);
        assert!(pending.locked_at_unix.is_none());

        let locked = by_sender.get("slack:U67890").expect("locked row");
        assert_eq!(locked.failures, 5);
        assert_eq!(locked.locked_at_unix, Some(1_700_000_000));

        backend.clear("slack:U67890").expect("clear");
        let rows = backend.load_all().expect("load after clear");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sender_id, "slack:U12345");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upsert_is_idempotent() {
        let dir = tempdir().unwrap();
        let backend = open_backend(&dir.path().join("kernel.surreal")).await;

        let row = TotpLockoutRow {
            sender_id: "channel/with/slashes".into(),
            failures: 1,
            locked_at_unix: None,
        };
        backend.upsert(&row).expect("first upsert");
        backend.upsert(&row).expect("second upsert");

        let rows = backend.load_all().expect("load");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sender_id, "channel/with/slashes");
        assert_eq!(rows[0].failures, 1);
    }
}
