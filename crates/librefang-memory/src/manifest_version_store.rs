//! SQLite-backed agent manifest version history (refs version-history feature).
//!
//! Every time an agent's manifest is persisted to disk the full TOML
//! snapshot is recorded here so operators can see what changed over time.
//! Read-only: there is no restore endpoint for agent manifests.
//!
//! Retention is per-agent, capped at [`MAX_VERSIONS_PER_AGENT`] most
//! recent snapshots. Trimmed on insert inside the same transaction.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;

/// Most recent manifest snapshots kept per agent.
pub const MAX_VERSIONS_PER_AGENT: usize = 50;

/// One recorded manifest snapshot.
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
/// Shares the connection pool every other store in `MemorySubstrate`
/// uses. The `manifest_versions` table is created by
/// `migration::migrate_v58`.
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
    /// Skips the insert when both the TOML and `change_source` are
    /// identical to the most recent stored version for this agent (avoids
    /// noise from no-op persists during boot reconciliation). Comparing
    /// `change_source` too means a `update-persist-failed` row never
    /// suppresses the next successful `update` of the same content — the
    /// disk-write failure this row exists to surface would otherwise stay
    /// the newest entry forever even after a successful retry.
    pub fn record_version(
        &self,
        agent_id: &str,
        agent_name: &str,
        manifest_toml: &str,
        change_source: &str,
    ) -> LibreFangResult<()> {
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        // `Immediate` takes the write lock up front, so two concurrent
        // `record_version` calls for the same agent cannot both observe
        // the same latest row and double-insert — the dedupe below holds
        // under concurrency rather than only in the happy path.
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

    /// Delete all manifest versions for an agent (cascade on agent removal).
    pub fn delete_for_agent(&self, agent_id: &str) -> LibreFangResult<usize> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "DELETE FROM manifest_versions WHERE agent_id = ?1",
            [agent_id],
        )
        .map_err(|e| {
            LibreFangError::memory_msg(format!("manifest version cascade delete failed: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn test_pool() -> Pool<SqliteConnectionManager> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
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

    #[test]
    fn delete_cascade() {
        let store = ManifestVersionStore::new(test_pool());
        store
            .record_version("a1", "agent", "v1", "dashboard")
            .unwrap();
        store
            .record_version("a1", "agent", "v2", "dashboard")
            .unwrap();
        let deleted = store.delete_for_agent("a1").unwrap();
        assert_eq!(deleted, 2);
        assert!(store.list_for_agent("a1", 10).unwrap().is_empty());
    }

    /// `delete_cascade` above only proves this store can clear its own table; removing
    /// an agent goes through `AGENT_SCOPED_TABLES` instead, and nothing at that delete
    /// site names `manifest_versions`.
    /// Drop the entry and every test here still passes while a deleted agent's manifest
    /// history survives it, with no error and no orphan the caller can see.
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

    /// A disk-full write records `update-persist-failed` with the
    /// in-memory TOML; the operator retries and the identical content now
    /// persists successfully as `update`. The dedup must not read that as
    /// "no change" and leave the failure row as the newest entry forever.
    #[test]
    fn identical_content_with_a_different_change_source_is_not_deduped() {
        let store = ManifestVersionStore::new(test_pool());
        store
            .record_version("a1", "agent", "same", "update-persist-failed")
            .unwrap();
        store
            .record_version("a1", "agent", "same", "update")
            .unwrap();

        let versions = store.list_for_agent("a1", 10).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].change_source, "update");
        assert_eq!(versions[1].change_source, "update-persist-failed");
    }

    /// A read failure on the dedup SELECT must surface as its own error,
    /// not get swallowed into "no previous version" and reappear as a
    /// misleading insert failure against a table that in fact exists.
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
}
