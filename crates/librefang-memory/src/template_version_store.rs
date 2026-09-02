//! SQLite-backed agent-type (template) version history.
//!
//! Every time an agent type is created or edited through the API the full
//! TOML snapshot is recorded here so operators can see what changed over
//! time and restore a prior configuration.
//!
//! Retention is per-template, capped at [`MAX_VERSIONS_PER_TEMPLATE`]
//! most recent snapshots.  Trimmed on insert inside the same transaction.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;

/// Most recent snapshots kept per template.
pub const MAX_VERSIONS_PER_TEMPLATE: usize = 50;

/// One recorded template snapshot.
#[derive(Debug, Clone)]
pub struct TemplateVersionRow {
    pub id: i64,
    pub template_name: String,
    pub timestamp: String,
    pub manifest_toml: String,
    pub change_source: String,
}

/// Persistent template-version store backed by SQLite.
///
/// Shares the connection pool every other store in `MemorySubstrate`
/// uses.  The `template_versions` table is created by
/// `migration::migrate_v55`.
#[derive(Clone)]
pub struct TemplateVersionStore {
    pool: Pool<SqliteConnectionManager>,
}

impl TemplateVersionStore {
    /// Wrap an existing connection pool.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Record one template snapshot and trim to [`MAX_VERSIONS_PER_TEMPLATE`].
    ///
    /// Skips the insert when the TOML is byte-identical to the most
    /// recent stored version for this template (avoids noise from
    /// no-op persists).
    pub fn record_version(
        &self,
        template_name: &str,
        manifest_toml: &str,
        change_source: &str,
    ) -> LibreFangResult<()> {
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        // `Immediate` takes the write lock up front, so two concurrent
        // `record_version` calls for the same template cannot both observe
        // the same latest row and double-insert — the dedupe below holds
        // under concurrency rather than only in the happy path.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        // Deduplicate: skip if the latest snapshot is byte-identical.
        // `.optional()` distinguishes "no rows" from a real read failure;
        // `.ok()` would read a missing table or I/O error as "no previous
        // version" and fall through to a much less useful insert error.
        let latest: Option<String> = tx
            .query_row(
                "SELECT manifest_toml FROM template_versions
                 WHERE template_name = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT 1",
                [template_name],
                |row| row.get(0),
            )
            .optional()
            .map_err(LibreFangError::memory)?;
        if latest.as_deref() == Some(manifest_toml) {
            return Ok(());
        }

        tx.execute(
            "INSERT INTO template_versions
                (template_name, manifest_toml, change_source)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![template_name, manifest_toml, change_source],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("template version insert failed: {e}")))?;

        // Trim to cap.
        tx.execute(
            "DELETE FROM template_versions
             WHERE template_name = ?1
               AND id NOT IN (
                   SELECT id FROM template_versions
                   WHERE template_name = ?1
                   ORDER BY timestamp DESC, id DESC
                   LIMIT ?2
               )",
            rusqlite::params![template_name, MAX_VERSIONS_PER_TEMPLATE as i64],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("template version trim failed: {e}")))?;

        tx.commit().map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// The most recent snapshots for a template, newest first.
    pub fn list_for_template(
        &self,
        template_name: &str,
        limit: usize,
    ) -> LibreFangResult<Vec<TemplateVersionRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, template_name, timestamp, manifest_toml, change_source
                 FROM template_versions
                 WHERE template_name = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![template_name, limit as i64], |row| {
                Ok(TemplateVersionRow {
                    id: row.get(0)?,
                    template_name: row.get(1)?,
                    timestamp: row.get(2)?,
                    manifest_toml: row.get(3)?,
                    change_source: row.get(4)?,
                })
            })
            .map_err(LibreFangError::memory)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(LibreFangError::memory)
    }

    /// Fetch a single version row by id.
    pub fn get_version(&self, version_id: i64) -> LibreFangResult<Option<TemplateVersionRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.query_row(
            "SELECT id, template_name, timestamp, manifest_toml, change_source
             FROM template_versions
             WHERE id = ?1",
            [version_id],
            |row| {
                Ok(TemplateVersionRow {
                    id: row.get(0)?,
                    template_name: row.get(1)?,
                    timestamp: row.get(2)?,
                    manifest_toml: row.get(3)?,
                    change_source: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(LibreFangError::memory)
    }

    /// Delete all versions for a template (cascade on template removal).
    pub fn delete_for_template(&self, template_name: &str) -> LibreFangResult<usize> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "DELETE FROM template_versions WHERE template_name = ?1",
            [template_name],
        )
        .map_err(|e| {
            LibreFangError::memory_msg(format!("template version cascade delete failed: {e}"))
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
        let store = TemplateVersionStore::new(test_pool());
        store
            .record_version("assistant", "name = \"v1\"", "dashboard")
            .unwrap();
        store
            .record_version("assistant", "name = \"v2\"", "api")
            .unwrap();

        let versions = store.list_for_template("assistant", 10).unwrap();
        assert_eq!(versions.len(), 2);
        // Newest first.
        assert_eq!(versions[0].manifest_toml, "name = \"v2\"");
        assert_eq!(versions[0].change_source, "api");
        assert_eq!(versions[1].manifest_toml, "name = \"v1\"");
    }

    #[test]
    fn deduplicates_identical_consecutive_writes() {
        let store = TemplateVersionStore::new(test_pool());
        store
            .record_version("assistant", "same", "dashboard")
            .unwrap();
        store
            .record_version("assistant", "same", "dashboard")
            .unwrap();

        let versions = store.list_for_template("assistant", 10).unwrap();
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn trims_to_cap() {
        let store = TemplateVersionStore::new(test_pool());
        for i in 0..(MAX_VERSIONS_PER_TEMPLATE + 10) {
            store
                .record_version("assistant", &format!("iteration = {i}"), "test")
                .unwrap();
        }
        let versions = store.list_for_template("assistant", 200).unwrap();
        assert_eq!(versions.len(), MAX_VERSIONS_PER_TEMPLATE);
    }

    #[test]
    fn delete_cascade() {
        let store = TemplateVersionStore::new(test_pool());
        store
            .record_version("assistant", "v1", "dashboard")
            .unwrap();
        store
            .record_version("assistant", "v2", "dashboard")
            .unwrap();
        let deleted = store.delete_for_template("assistant").unwrap();
        assert_eq!(deleted, 2);
        assert!(store.list_for_template("assistant", 10).unwrap().is_empty());
    }

    #[test]
    fn get_version_by_id() {
        let store = TemplateVersionStore::new(test_pool());
        store
            .record_version("assistant", "v1", "dashboard")
            .unwrap();

        let versions = store.list_for_template("assistant", 10).unwrap();
        let id = versions[0].id;

        let row = store.get_version(id).unwrap().unwrap();
        assert_eq!(row.manifest_toml, "v1");
        assert_eq!(row.template_name, "assistant");

        assert!(store.get_version(99999).unwrap().is_none());
    }

    #[test]
    fn separate_templates_are_independent() {
        let store = TemplateVersionStore::new(test_pool());
        store.record_version("alpha", "a1", "dashboard").unwrap();
        store.record_version("beta", "b1", "dashboard").unwrap();

        assert_eq!(store.list_for_template("alpha", 10).unwrap().len(), 1);
        assert_eq!(store.list_for_template("beta", 10).unwrap().len(), 1);

        store.delete_for_template("alpha").unwrap();
        assert!(store.list_for_template("alpha", 10).unwrap().is_empty());
        assert_eq!(store.list_for_template("beta", 10).unwrap().len(), 1);
    }
}
