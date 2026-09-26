//! SQLite-backed agent manifest version history.
//!
//! Every time an agent's manifest is persisted to `agent.toml` the full TOML
//! snapshot is recorded here so operators can see what changed over time and
//! restore a prior configuration from the dashboard.
//!
//! Retention is per-agent, capped at [`MAX_VERSIONS_PER_AGENT`] most recent
//! snapshots. Trimmed on insert inside the same transaction.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;

/// Most recent manifest snapshots kept per agent.
pub const MAX_VERSIONS_PER_AGENT: usize = 50;

/// One recorded manifest snapshot.
///
/// `agent_name` is denormalised on purpose: it is the name at snapshot time,
/// so a rename leaves old rows carrying the historical name.
/// `change_source` is a short tag naming what wrote the snapshot; each
/// `persist_manifest_to_disk` call site in the kernel passes its own (see
/// [`ManifestVersionStore::record_version`]). The schema default `unknown`
/// covers rows written by a writer that does not classify its persist.
#[derive(Debug, Clone)]
pub struct ManifestVersionRow {
    pub id: i64,
    pub agent_id: String,
    pub agent_name: String,
    pub timestamp: String,
    pub manifest_toml: String,
    pub change_source: String,
}

/// Persistent manifest-version store backed by SQLite.
///
/// Shares the connection pool every other store in `MemorySubstrate` uses.
/// The `manifest_versions` table is created by `migration::migrate_v58`, and
/// its rows are purged on agent removal through `AGENT_SCOPED_TABLES`.
#[derive(Clone)]
pub struct ManifestVersionStore {
    pool: Pool<SqliteConnectionManager>,
}

impl ManifestVersionStore {
    /// Wrap an existing connection pool.
    ///
    /// The caller must ensure `migration::run_migrations` has already
    /// executed so the `manifest_versions` table exists.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Record one manifest snapshot and trim the agent back to
    /// [`MAX_VERSIONS_PER_AGENT`].
    ///
    /// Skips the insert when both the TOML and `change_source` are identical
    /// to the most recent stored version for this agent (avoids noise from
    /// no-op persists during boot reconciliation). Comparing `change_source`
    /// too means a `restore` of content identical to the current version is
    /// still recorded as its own event.
    pub fn record_version(
        &self,
        agent_id: &str,
        agent_name: &str,
        manifest_toml: &str,
        change_source: &str,
    ) -> LibreFangResult<()> {
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        // `Immediate` takes the write lock up front, so two concurrent
        // `record_version` calls for the same agent cannot both observe the
        // same latest row and double-insert — the dedupe below holds under
        // concurrency rather than only in the happy path.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        // Deduplicate: skip if the latest snapshot matches on both fields.
        // `.optional()` distinguishes "no rows" from a real read failure;
        // `.ok()` would read a missing table or I/O error as "no previous
        // version" and fall through to a much less useful insert error.
        let latest: Option<(String, String)> = tx
            .query_row(
                "SELECT manifest_toml, change_source FROM manifest_versions
                 WHERE agent_id = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT 1",
                [agent_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| {
                LibreFangError::memory_msg(format!("manifest version dedup read failed: {e}"))
            })?;
        if latest.as_ref().map(|(t, s)| (t.as_str(), s.as_str()))
            == Some((manifest_toml, change_source))
        {
            tx.commit().map_err(LibreFangError::memory)?;
            return Ok(());
        }

        tx.execute(
            "INSERT INTO manifest_versions
                (agent_id, agent_name, manifest_toml, change_source)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![agent_id, agent_name, manifest_toml, change_source],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("manifest version insert failed: {e}")))?;

        // Trim to cap.
        tx.execute(
            "DELETE FROM manifest_versions
             WHERE agent_id = ?1
               AND id NOT IN (
                   SELECT id FROM manifest_versions
                   WHERE agent_id = ?1
                   ORDER BY timestamp DESC, id DESC
                   LIMIT ?2
               )",
            rusqlite::params![agent_id, MAX_VERSIONS_PER_AGENT as i64],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("manifest version trim failed: {e}")))?;

        tx.commit().map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// The most recent manifest snapshots for an agent, newest first.
    pub fn list_for_agent(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> LibreFangResult<Vec<ManifestVersionRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, agent_id, agent_name, timestamp, manifest_toml, change_source
                 FROM manifest_versions
                 WHERE agent_id = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id, limit as i64], |row| {
                Ok(ManifestVersionRow {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    agent_name: row.get(2)?,
                    timestamp: row.get(3)?,
                    manifest_toml: row.get(4)?,
                    change_source: row.get(5)?,
                })
            })
            .map_err(LibreFangError::memory)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(LibreFangError::memory)
    }

    /// Fetch a single version row by id (backing the restore endpoint).
    pub fn get_version(&self, version_id: i64) -> LibreFangResult<Option<ManifestVersionRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.query_row(
            "SELECT id, agent_id, agent_name, timestamp, manifest_toml, change_source
             FROM manifest_versions
             WHERE id = ?1",
            [version_id],
            |row| {
                Ok(ManifestVersionRow {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    agent_name: row.get(2)?,
                    timestamp: row.get(3)?,
                    manifest_toml: row.get(4)?,
                    change_source: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(LibreFangError::memory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;
    use std::sync::{Arc, Barrier};

    fn test_pool() -> Pool<SqliteConnectionManager> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        pool
    }

    /// File-backed, multi-connection pool so threads genuinely contend for
    /// the SQLite write lock — an in-memory `:memory:` DB pinned to
    /// `max_size(1)` cannot exercise the race the `Immediate` transaction
    /// exists for. Pragmas mirror production (`WAL`), or commit-time lock
    /// promotion returns `SQLITE_BUSY` without the busy handler ever running.
    fn test_file_pool(path: &std::path::Path) -> Pool<SqliteConnectionManager> {
        let pool = Pool::builder()
            .max_size(8)
            .build(SqliteConnectionManager::file(path).with_init(|c| {
                c.execute_batch(crate::substrate::DEFAULT_CONNECTION_PRAGMAS)?;
                c.busy_timeout(std::time::Duration::from_secs(30))
            }))
            .unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        pool
    }

    #[test]
    fn record_and_list_versions() {
        let store = ManifestVersionStore::new(test_pool());
        store
            .record_version("a1", "test-agent", "name = \"v1\"", "dashboard")
            .unwrap();
        store
            .record_version("a1", "test-agent", "name = \"v2\"", "api")
            .unwrap();

        let versions = store.list_for_agent("a1", 10).unwrap();
        assert_eq!(versions.len(), 2);
        // Newest first.
        assert_eq!(versions[0].manifest_toml, "name = \"v2\"");
        assert_eq!(versions[0].change_source, "api");
        assert_eq!(versions[1].manifest_toml, "name = \"v1\"");
    }

    #[test]
    fn deduplicates_identical_consecutive_writes() {
        let store = ManifestVersionStore::new(test_pool());
        store.record_version("a1", "agent", "same", "boot").unwrap();
        store.record_version("a1", "agent", "same", "boot").unwrap();

        let versions = store.list_for_agent("a1", 10).unwrap();
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn trims_to_cap() {
        let store = ManifestVersionStore::new(test_pool());
        for i in 0..(MAX_VERSIONS_PER_AGENT + 10) {
            store
                .record_version("a1", "agent", &format!("iteration = {i}"), "test")
                .unwrap();
        }
        let versions = store.list_for_agent("a1", 200).unwrap();
        assert_eq!(versions.len(), MAX_VERSIONS_PER_AGENT);
    }

    /// Four threads record the same content for the same agent at once. The
    /// `Immediate` transaction serialises them, so exactly one row lands: the
    /// check-then-insert dedupe would double-insert if it ran the SELECT
    /// outside a write lock.
    #[test]
    fn concurrent_identical_writes_record_one_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ManifestVersionStore::new(test_file_pool(&tmp.path().join("manifest.db")));

        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let store = store.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store.record_version("a1", "agent", "same", "api")
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }

        let versions = store.list_for_agent("a1", 10).unwrap();
        assert_eq!(versions.len(), 1, "{versions:?}");
    }

    #[test]
    fn get_version_by_id() {
        let store = ManifestVersionStore::new(test_pool());
        store
            .record_version("a1", "agent", "v1", "dashboard")
            .unwrap();
        let id = store.list_for_agent("a1", 10).unwrap()[0].id;

        let row = store.get_version(id).unwrap().unwrap();
        assert_eq!(row.manifest_toml, "v1");
        assert_eq!(row.agent_id, "a1");

        assert!(store.get_version(99999).unwrap().is_none());
    }

    /// A read failure on the dedup SELECT must surface as its own error, not
    /// get swallowed into "no previous version" and reappear as a misleading
    /// insert failure against a table that in fact exists.
    #[test]
    fn dedup_read_failure_is_reported_as_a_read_failure() {
        let store = ManifestVersionStore::new(test_pool());
        // Drop the table out from under the store to force the SELECT to
        // fail with something other than "no rows".
        store
            .pool
            .get()
            .unwrap()
            .execute("DROP TABLE manifest_versions", [])
            .unwrap();

        let err = store
            .record_version("a1", "agent", "v1", "dashboard")
            .unwrap_err();
        assert!(
            err.to_string().contains("dedup read failed"),
            "expected the dedup read's own error, got: {err}"
        );
    }

    /// Removing an agent goes through `AGENT_SCOPED_TABLES`, and nothing at
    /// that delete site names `manifest_versions`. Drop the entry and every
    /// test here still passes while a deleted agent's manifest history
    /// survives it, with no error and no orphan the caller can see.
    #[test]
    fn remove_agent_purges_manifest_versions() {
        let pool = test_pool();
        let store = ManifestVersionStore::new(pool.clone());
        let agent = librefang_types::agent::AgentId(uuid::Uuid::new_v4());
        let agent_id = agent.0.to_string();

        store
            .record_version(&agent_id, "agent", "name = \"v1\"", "dashboard")
            .unwrap();
        assert_eq!(store.list_for_agent(&agent_id, 10).unwrap().len(), 1);

        crate::structured::StructuredStore::new(pool)
            .remove_agent(agent)
            .unwrap();

        assert!(store.list_for_agent(&agent_id, 10).unwrap().is_empty());
    }
}
