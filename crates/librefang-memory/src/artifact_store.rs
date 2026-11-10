//! Versioned, content-addressed session artifacts.
//!
//! Every row names one version of a caller-supplied path for a session. Bytes
//! live once under `<blob_root>/blobs/<first2>/<sha256>` and are shared across
//! sessions; a new version is recorded only when the content hash differs from
//! the latest version of that `(session, path)`.

use chrono::{DateTime, Duration, Utc};
use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One recorded version of a session artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    /// Row id (UUID).
    pub id: String,
    /// Session the artifact belongs to.
    pub session_id: String,
    /// Message that produced this version, when there was one.
    pub message_id: Option<String>,
    /// Caller-normalised path, stored as given.
    pub path: String,
    /// SHA-256 of the blob, lowercase hex.
    pub content_hash: String,
    /// Blob size in bytes.
    pub size_bytes: i64,
    /// MIME type as reported by the caller.
    pub mime_type: Option<String>,
    /// Monotonic version per `(session, path)`, starting at 1.
    pub version: i64,
    /// Row creation time (`YYYY-MM-DD HH:MM:SS`, UTC).
    pub created_at: String,
}

/// Outcome of one [`ArtifactStore::prune`] run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneReport {
    /// Artifact rows removed.
    pub rows_deleted: usize,
    /// Unreferenced blob files removed.
    pub blobs_deleted: usize,
}

/// Persistent artifact store backed by SQLite plus a blob directory.
///
/// Shares the connection pool every other store in `MemorySubstrate` uses.
/// The `artifacts` table is created by `migration::migrate_v66`.
#[derive(Clone)]
pub struct ArtifactStore {
    pool: Pool<SqliteConnectionManager>,
    blob_root: PathBuf,
    /// Serialises `put` against `prune` inside this process. Without it a blob
    /// written by `put` can be swept between its write and its row insert.
    /// A second daemon pointed at the same blob directory is out of scope.
    op_lock: Arc<Mutex<()>>,
}

const RECORD_COLUMNS: &str =
    "id, session_id, message_id, path, content_hash, size_bytes, mime_type, version, created_at";

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        message_id: row.get(2)?,
        path: row.get(3)?,
        content_hash: row.get(4)?,
        size_bytes: row.get(5)?,
        mime_type: row.get(6)?,
        version: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl ArtifactStore {
    /// Wrap an existing connection pool and blob root.
    ///
    /// The caller must ensure `migration::run_migrations` has already run.
    pub fn new(pool: Pool<SqliteConnectionManager>, blob_root: PathBuf) -> Self {
        Self {
            pool,
            blob_root,
            op_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Store `bytes` as the next version of `(session_id, path)`.
    ///
    /// Identical content returns the existing latest row without inserting;
    /// the blob is written before the row so a failed insert leaves only an
    /// orphan blob, which the next prune removes.
    pub fn put(
        &self,
        session_id: &str,
        message_id: Option<&str>,
        path: &str,
        bytes: &[u8],
        mime_type: Option<&str>,
    ) -> LibreFangResult<ArtifactRecord> {
        let _guard = self.op_lock.lock().expect("artifact op lock poisoned");
        let hash = hex_sha256(bytes);

        if let Some(latest) = self.latest(session_id, path)? {
            if latest.content_hash == hash {
                return Ok(latest);
            }
        }

        self.write_blob(&hash, bytes)?;

        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;
        // `Immediate` serialises concurrent puts for the same path, so the
        // version number cannot be chosen twice from the same latest row.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        if let Some(latest) =
            Self::latest_in(&tx, session_id, path).map_err(LibreFangError::memory)?
        {
            if latest.content_hash == hash {
                tx.commit().map_err(LibreFangError::memory)?;
                return Ok(latest);
            }
        }

        let latest_version: Option<i64> = tx
            .query_row(
                "SELECT MAX(version) FROM artifacts WHERE session_id = ?1 AND path = ?2",
                rusqlite::params![session_id, path],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        let version = latest_version.unwrap_or(0) + 1;
        let id = uuid::Uuid::new_v4().to_string();

        tx.execute(
            "INSERT INTO artifacts
                (id, session_id, message_id, path, content_hash, size_bytes, mime_type, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                session_id,
                message_id,
                path,
                hash,
                bytes.len() as i64,
                mime_type,
                version,
            ],
        )
        .map_err(LibreFangError::memory)?;

        let record = tx
            .query_row(
                &format!("SELECT {RECORD_COLUMNS} FROM artifacts WHERE id = ?1"),
                [&id],
                row_to_record,
            )
            .map_err(LibreFangError::memory)?;

        tx.commit().map_err(LibreFangError::memory)?;
        Ok(record)
    }

    /// Latest version of every path in a session, newest first.
    pub fn list(&self, session_id: &str) -> LibreFangResult<Vec<ArtifactRecord>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {RECORD_COLUMNS} FROM artifacts a
                 WHERE a.session_id = ?1
                   AND a.version = (
                       SELECT MAX(b.version) FROM artifacts b
                       WHERE b.session_id = a.session_id AND b.path = a.path
                   )
                 ORDER BY a.created_at DESC, a.rowid DESC"
            ))
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([session_id], row_to_record)
            .map_err(LibreFangError::memory)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(LibreFangError::memory)
    }

    /// Every stored version of one path, oldest first.
    pub fn versions(&self, session_id: &str, path: &str) -> LibreFangResult<Vec<ArtifactRecord>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {RECORD_COLUMNS} FROM artifacts
                 WHERE session_id = ?1 AND path = ?2
                 ORDER BY version ASC"
            ))
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![session_id, path], row_to_record)
            .map_err(LibreFangError::memory)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(LibreFangError::memory)
    }

    /// Fetch one version and its bytes. `None` means no such row.
    pub fn read(
        &self,
        session_id: &str,
        path: &str,
        version: i64,
    ) -> LibreFangResult<Option<(ArtifactRecord, Vec<u8>)>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let record = conn
            .query_row(
                &format!(
                    "SELECT {RECORD_COLUMNS} FROM artifacts
                     WHERE session_id = ?1 AND path = ?2 AND version = ?3"
                ),
                rusqlite::params![session_id, path, version],
                row_to_record,
            )
            .optional()
            .map_err(LibreFangError::memory)?;
        drop(conn);

        let Some(record) = record else {
            return Ok(None);
        };
        let bytes = fs::read(self.blob_path(&record.content_hash)).map_err(|e| {
            LibreFangError::memory_msg(format!(
                "artifact blob {} unreadable: {e}",
                record.content_hash
            ))
        })?;
        Ok(Some((record, bytes)))
    }

    /// Delete rows older than `retention_days`, then blobs no row references.
    ///
    /// `retention_days == 0` keeps everything and touches no files.
    pub fn prune(&self, retention_days: u32, now: DateTime<Utc>) -> LibreFangResult<PruneReport> {
        if retention_days == 0 {
            return Ok(PruneReport::default());
        }

        let _guard = self.op_lock.lock().expect("artifact op lock poisoned");
        let cutoff = (now - Duration::days(i64::from(retention_days)))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let rows_deleted = conn
            .execute("DELETE FROM artifacts WHERE created_at < ?1", [&cutoff])
            .map_err(LibreFangError::memory)?;

        let mut stmt = conn
            .prepare("SELECT DISTINCT content_hash FROM artifacts")
            .map_err(LibreFangError::memory)?;
        let referenced: HashSet<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(LibreFangError::memory)?
            .collect::<Result<_, _>>()
            .map_err(LibreFangError::memory)?;
        drop(stmt);
        drop(conn);

        let blobs_deleted = self.delete_unreferenced_blobs(&referenced)?;
        Ok(PruneReport {
            rows_deleted,
            blobs_deleted,
        })
    }

    fn latest(&self, session_id: &str, path: &str) -> LibreFangResult<Option<ArtifactRecord>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        Self::latest_in(&conn, session_id, path).map_err(LibreFangError::memory)
    }

    fn latest_in(
        conn: &rusqlite::Connection,
        session_id: &str,
        path: &str,
    ) -> rusqlite::Result<Option<ArtifactRecord>> {
        conn.query_row(
            &format!(
                "SELECT {RECORD_COLUMNS} FROM artifacts
                 WHERE session_id = ?1 AND path = ?2
                 ORDER BY version DESC
                 LIMIT 1"
            ),
            rusqlite::params![session_id, path],
            row_to_record,
        )
        .optional()
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        self.blob_root.join("blobs").join(&hash[..2]).join(hash)
    }

    fn write_blob(&self, hash: &str, bytes: &[u8]) -> LibreFangResult<()> {
        let path = self.blob_path(hash);
        if path.exists() {
            return Ok(());
        }

        let dir = path
            .parent()
            .expect("content-addressed blob path always has a parent");
        fs::create_dir_all(dir).map_err(|e| {
            LibreFangError::memory_msg(format!("artifact blob dir {}: {e}", dir.display()))
        })?;

        let tmp = dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        if let Err(e) = fs::write(&tmp, bytes) {
            let _ = fs::remove_file(&tmp);
            return Err(LibreFangError::memory_msg(format!(
                "artifact blob write {}: {e}",
                tmp.display()
            )));
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            // A concurrent writer may have completed the same hash first.
            if path.exists() {
                return Ok(());
            }
            return Err(LibreFangError::memory_msg(format!(
                "artifact blob rename {}: {e}",
                path.display()
            )));
        }
        Ok(())
    }

    fn delete_unreferenced_blobs(&self, referenced: &HashSet<String>) -> LibreFangResult<usize> {
        // `put` and `prune` take `op_lock` in this process, so a fresh blob
        // cannot be swept between its write and its row insert. A second
        // daemon sharing the same blob directory is not covered.
        let root = self.blob_root.join("blobs");
        if !root.exists() {
            return Ok(0);
        }

        let mut deleted = 0;
        for prefix in fs::read_dir(&root).map_err(|e| {
            LibreFangError::memory_msg(format!("artifact blob dir {}: {e}", root.display()))
        })? {
            let prefix = prefix.map_err(LibreFangError::memory)?;
            if !prefix.path().is_dir() {
                continue;
            }
            for entry in fs::read_dir(prefix.path()).map_err(LibreFangError::memory)? {
                let entry = entry.map_err(LibreFangError::memory)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                // Only content-addressed names are eligible: a temp file from
                // an in-flight put must not be swept.
                if name.len() == 64
                    && name.bytes().all(|b| b.is_ascii_hexdigit())
                    && !referenced.contains(&name)
                {
                    fs::remove_file(entry.path()).map_err(LibreFangError::memory)?;
                    deleted += 1;
                }
            }
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_backed_pool;
    use chrono::TimeZone;
    use std::path::PathBuf;

    struct Harness {
        _tmp: tempfile::TempDir,
        store: ArtifactStore,
        db: PathBuf,
    }

    fn harness() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("memory.db");
        let store = ArtifactStore::new(file_backed_pool(&db), tmp.path().join("artifacts"));
        Harness {
            _tmp: tmp,
            store,
            db,
        }
    }

    fn blob_count(store: &ArtifactStore) -> usize {
        let root = store.blob_root.join("blobs");
        if !root.exists() {
            return 0;
        }
        fs::read_dir(root)
            .unwrap()
            .flat_map(|dir| fs::read_dir(dir.unwrap().path()).unwrap())
            .filter(|entry| entry.as_ref().unwrap().path().is_file())
            .count()
    }

    fn backdate(h: &Harness, id: &str) {
        let conn = rusqlite::Connection::open(&h.db).unwrap();
        conn.execute(
            "UPDATE artifacts SET created_at = '2020-01-01 00:00:00' WHERE id = ?1",
            [id],
        )
        .unwrap();
    }

    #[test]
    fn first_put_writes_row_and_blob() {
        let h = harness();
        let rec = h
            .store
            .put(
                "s1",
                Some("m1"),
                "notes/a.md",
                b"hello",
                Some("text/markdown"),
            )
            .unwrap();

        assert_eq!(rec.version, 1);
        assert_eq!(rec.session_id, "s1");
        assert_eq!(rec.message_id.as_deref(), Some("m1"));
        assert_eq!(rec.size_bytes, 5);
        assert_eq!(
            fs::read(h.store.blob_path(&rec.content_hash)).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn identical_bytes_insert_one_row() {
        let h = harness();
        let first = h.store.put("s1", None, "a.txt", b"same", None).unwrap();
        let second = h.store.put("s1", None, "a.txt", b"same", None).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(h.store.versions("s1", "a.txt").unwrap().len(), 1);
        assert_eq!(blob_count(&h.store), 1);
    }

    #[test]
    fn changed_bytes_create_version_two_and_keep_both_blobs() {
        let h = harness();
        let v1 = h.store.put("s1", None, "a.txt", b"one", None).unwrap();
        let v2 = h.store.put("s1", None, "a.txt", b"two", None).unwrap();

        assert_eq!(v2.version, 2);
        assert_ne!(v1.content_hash, v2.content_hash);
        assert!(h.store.blob_path(&v1.content_hash).exists());
        assert!(h.store.blob_path(&v2.content_hash).exists());
        assert_eq!(blob_count(&h.store), 2);
    }

    #[test]
    fn sessions_sharing_bytes_share_one_blob() {
        let h = harness();
        let a = h.store.put("s1", None, "a.txt", b"shared", None).unwrap();
        let b = h.store.put("s2", None, "b.txt", b"shared", None).unwrap();

        assert_eq!(a.content_hash, b.content_hash);
        assert_ne!(a.id, b.id);
        assert_eq!(blob_count(&h.store), 1);
    }

    #[test]
    fn prune_with_unlimited_retention_keeps_everything() {
        let h = harness();
        let rec = h.store.put("s1", None, "a.txt", b"keep", None).unwrap();

        let report = h.store.prune(0, Utc::now()).unwrap();

        assert_eq!(report, PruneReport::default());
        assert_eq!(h.store.versions("s1", "a.txt").unwrap().len(), 1);
        assert!(h.store.blob_path(&rec.content_hash).exists());
    }

    #[test]
    fn prune_removes_old_rows_and_unreferenced_blobs() {
        let h = harness();
        let old = h.store.put("s1", None, "a.txt", b"old", None).unwrap();
        let new = h.store.put("s1", None, "a.txt", b"new", None).unwrap();
        let other = h.store.put("s2", None, "b.txt", b"other", None).unwrap();
        backdate(&h, &old.id);
        backdate(&h, &other.id);

        let now = Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap();
        let report = h.store.prune(1, now).unwrap();

        assert_eq!(report.rows_deleted, 2);
        assert_eq!(report.blobs_deleted, 2);
        assert!(!h.store.blob_path(&old.content_hash).exists());
        assert!(!h.store.blob_path(&other.content_hash).exists());
        assert!(h.store.blob_path(&new.content_hash).exists());
        assert_eq!(h.store.versions("s1", "a.txt").unwrap().len(), 1);
    }

    #[test]
    fn read_returns_bytes_for_exact_version() {
        let h = harness();
        h.store.put("s1", None, "a.txt", b"first", None).unwrap();
        h.store.put("s1", None, "a.txt", b"second", None).unwrap();

        let (rec, bytes) = h.store.read("s1", "a.txt", 1).unwrap().unwrap();
        assert_eq!(rec.version, 1);
        assert_eq!(bytes, b"first");
        assert!(h.store.read("s1", "a.txt", 9).unwrap().is_none());
        assert!(h.store.read("s2", "a.txt", 1).unwrap().is_none());
    }

    #[test]
    fn list_returns_latest_per_path_newest_first() {
        let h = harness();
        h.store.put("s1", None, "b.txt", b"b1", None).unwrap();
        h.store.put("s1", None, "a.txt", b"a1", None).unwrap();
        h.store.put("s1", None, "b.txt", b"b2", None).unwrap();

        let rows = h.store.list("s1").unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "b.txt");
        assert_eq!(rows[0].version, 2);
        assert_eq!(rows[1].path, "a.txt");
        assert_eq!(rows[1].version, 1);
    }
}
