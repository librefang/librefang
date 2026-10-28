//! SQLite-backed skill version history (#8021).
//!
//! Every time a skill's `skill.toml` + `prompt_context.md` change — the
//! dashboard evolve endpoints, the agent-facing `skill_evolve_*` tools, the
//! skill-workshop approve/reviewer paths, installs/uninstalls and history
//! restores — a full snapshot of both files is recorded here so operators can
//! see how a skill changed over time and restore a prior state.
//!
//! Scope of v1: the key is `skill_name` alone, because only skills installed
//! under the global `~/.librefang/skills/` directory are tracked. Workspace
//! skills (agent mission workspaces) and hand-scoped skills live under
//! different roots and are deliberately not recorded — the name alone would
//! conflate two different skills shadowing each other. A future schema can add
//! a scope column without disturbing this one.
//!
//! The marketplace install paths (`clawhub.rs`, `skillhub.rs`) record an
//! `install` version once the skill lands globally. Direct CLI/TUI edits are
//! the only in-scope write path that remains unrecorded (no daemon funnel yet).
//!
//! Retention is per-skill, trimmed on insert inside the same transaction.
//! Content-changing rows are capped at [`MAX_VERSIONS_PER_SKILL`]; the
//! event-only sources (`file-write`, `file-remove`, `restore`) have their own
//! [`MAX_EVENT_ROWS`] cap and are pruned first, so supporting-file chatter
//! cannot evict restorable content. Deduplication compares `(manifest_toml,
//! prompt_context, change_source)`: a no-op persist that re-writes the same
//! bytes under the same source is skipped. Event-only sources are exempt:
//! their snapshot does not change, so the row is the event, and consecutive
//! identical rows must both land.

use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::skill_history::SkillWrite;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;

/// Most recent content-changing snapshots kept per skill.
pub const MAX_VERSIONS_PER_SKILL: usize = 50;

/// Most recent event-only rows (`file-write`, `file-remove`, `restore`) kept
/// per skill.
///
/// Counted separately from [`MAX_VERSIONS_PER_SKILL`]: an agent that writes
/// supporting files fifty times must not evict the content snapshots those
/// writes happened at.
pub const MAX_EVENT_ROWS: usize = 50;

/// Sources whose row records an event rather than a new content state: they
/// are exempt from deduplication and capped by [`MAX_EVENT_ROWS`].
const EVENT_SOURCES: [SkillWrite; 3] = [
    SkillWrite::FileWrite,
    SkillWrite::FileRemove,
    SkillWrite::Restore,
];

/// One recorded skill snapshot.
///
/// `skill_name` is the key; `manifest_toml` and `prompt_context` together are
/// the full snapshot (supporting files and wasm runtimes are out of scope in
/// v1). `author` names the surface that made the change (`dashboard`, a user
/// name, `agent:<id>`, `reviewer:agent:<id>`), and `change_source` is the
/// closed operation vocabulary — see
/// [`librefang_types::skill_history::SkillWrite`], the canonical definition
/// of that set.
#[derive(Debug, Clone)]
pub struct SkillVersionRow {
    pub id: i64,
    pub skill_name: String,
    pub timestamp: String,
    pub manifest_toml: String,
    pub prompt_context: String,
    pub author: String,
    pub change_source: String,
}

/// Persistent skill-version store backed by SQLite.
///
/// Shares the connection pool every other store in `MemorySubstrate` uses.
/// The `skill_versions` table is created by `migration::migrate_v65`.
#[derive(Clone)]
pub struct SkillVersionStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SkillVersionStore {
    /// Wrap an existing connection pool.
    ///
    /// The caller must ensure `migration::run_migrations` has already executed
    /// so the `skill_versions` table exists.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Record one skill snapshot and trim the skill back to
    /// [`MAX_VERSIONS_PER_SKILL`] content rows plus [`MAX_EVENT_ROWS`] event
    /// rows.
    ///
    /// Skips the insert when the manifest, the prompt context and the
    /// `change_source` are all identical to the most recent stored version for
    /// this skill. Event-only sources (`file-write`, `file-remove`, `restore`)
    /// are exempt and always insert, because their snapshot is unchanged by
    /// definition and the row records the event itself.
    pub fn record_version(
        &self,
        skill_name: &str,
        manifest_toml: &str,
        prompt_context: &str,
        author: &str,
        change_source: &str,
    ) -> LibreFangResult<()> {
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        // `Immediate` takes the write lock up front, so two concurrent
        // `record_version` calls for the same skill cannot both observe the
        // same latest row and double-insert — the dedupe below holds under
        // concurrency rather than only in the happy path.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        // Deduplicate: skip if the latest snapshot matches on all three fields.
        // `.optional()` distinguishes "no rows" from a real read failure;
        // `.ok()` would read a missing table or I/O error as "no previous
        // version" and fall through to a much less useful insert error.
        //
        // Event-only sources are exempt: a supporting-file write/remove does
        // not change the v1 snapshot, and a restore of content identical to
        // the current state is still its own event, so the row is the only
        // record. Deduping them would collapse consecutive events into one.
        let event_only = EVENT_SOURCES
            .iter()
            .any(|write| write.as_str() == change_source);
        let latest: Option<(String, String, String)> = tx
            .query_row(
                "SELECT manifest_toml, prompt_context, change_source FROM skill_versions
                 WHERE skill_name = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT 1",
                [skill_name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| {
                LibreFangError::memory_msg(format!("skill version dedup read failed: {e}"))
            })?;
        if !event_only
            && latest
                .as_ref()
                .map(|(t, p, s)| (t.as_str(), p.as_str(), s.as_str()))
                == Some((manifest_toml, prompt_context, change_source))
        {
            tx.commit().map_err(LibreFangError::memory)?;
            return Ok(());
        }

        tx.execute(
            "INSERT INTO skill_versions
                (skill_name, manifest_toml, prompt_context, author, change_source)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                skill_name,
                manifest_toml,
                prompt_context,
                author,
                change_source
            ],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("skill version insert failed: {e}")))?;

        // Trim to cap, event rows first. The two caps apply to disjoint sets:
        // an event row's unchanged snapshot is kept in its own small budget
        // rather than spending a content slot the operator could restore.
        let event_sources = EVENT_SOURCES.map(SkillWrite::as_str);
        tx.execute(
            "DELETE FROM skill_versions
             WHERE skill_name = ?1
               AND change_source IN (?2, ?3, ?4)
               AND id NOT IN (
                   SELECT id FROM skill_versions
                   WHERE skill_name = ?1
                     AND change_source IN (?2, ?3, ?4)
                   ORDER BY timestamp DESC, id DESC
                   LIMIT ?5
               )",
            rusqlite::params![
                skill_name,
                event_sources[0],
                event_sources[1],
                event_sources[2],
                MAX_EVENT_ROWS as i64
            ],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("skill version trim failed: {e}")))?;
        tx.execute(
            "DELETE FROM skill_versions
             WHERE skill_name = ?1
               AND change_source NOT IN (?2, ?3, ?4)
               AND id NOT IN (
                   SELECT id FROM skill_versions
                   WHERE skill_name = ?1
                     AND change_source NOT IN (?2, ?3, ?4)
                   ORDER BY timestamp DESC, id DESC
                   LIMIT ?5
               )",
            rusqlite::params![
                skill_name,
                event_sources[0],
                event_sources[1],
                event_sources[2],
                MAX_VERSIONS_PER_SKILL as i64
            ],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("skill version trim failed: {e}")))?;

        tx.commit().map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// The most recent snapshots for a skill, newest first.
    pub fn list_for_skill(
        &self,
        skill_name: &str,
        limit: usize,
    ) -> LibreFangResult<Vec<SkillVersionRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, skill_name, timestamp, manifest_toml, prompt_context, author,
                        change_source
                 FROM skill_versions
                 WHERE skill_name = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![skill_name, limit as i64], |row| {
                Ok(SkillVersionRow {
                    id: row.get(0)?,
                    skill_name: row.get(1)?,
                    timestamp: row.get(2)?,
                    manifest_toml: row.get(3)?,
                    prompt_context: row.get(4)?,
                    author: row.get(5)?,
                    change_source: row.get(6)?,
                })
            })
            .map_err(LibreFangError::memory)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(LibreFangError::memory)
    }

    /// Fetch a single version row by id (backing the restore endpoint).
    pub fn get_version(&self, version_id: i64) -> LibreFangResult<Option<SkillVersionRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.query_row(
            "SELECT id, skill_name, timestamp, manifest_toml, prompt_context, author,
                    change_source
             FROM skill_versions
             WHERE id = ?1",
            [version_id],
            |row| {
                Ok(SkillVersionRow {
                    id: row.get(0)?,
                    skill_name: row.get(1)?,
                    timestamp: row.get(2)?,
                    manifest_toml: row.get(3)?,
                    prompt_context: row.get(4)?,
                    author: row.get(5)?,
                    change_source: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(LibreFangError::memory)
    }

    /// Whether a `delete` or `uninstall` row was recorded after `version_id`.
    ///
    /// History is keyed by skill name alone, so a re-install under the same
    /// name continues the old story: restoring a row that predates the
    /// removal would write a manifest whose `runtime` / `source` provenance
    /// belongs to the directory that was removed, not this one.
    pub fn removal_recorded_after(
        &self,
        skill_name: &str,
        version_id: i64,
    ) -> LibreFangResult<bool> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM skill_versions
                 WHERE skill_name = ?1
                   AND id > ?2
                   AND change_source IN (?3, ?4)
             )",
            rusqlite::params![
                skill_name,
                version_id,
                SkillWrite::Delete.as_str(),
                SkillWrite::Uninstall.as_str()
            ],
            |row| row.get(0),
        )
        .map_err(|e| LibreFangError::memory_msg(format!("skill version removal read failed: {e}")))
    }

    /// Delete all versions for a skill.
    ///
    /// Kept as an explicit cascade API (mirroring
    /// [`crate::TemplateVersionStore::delete_for_template`]) rather than
    /// wired into delete/uninstall: the history *is* the record that a skill
    /// was deleted, and a later reinstall under the same name inherits the
    /// story rather than starting blank. A future purge endpoint can call this.
    pub fn delete_for_skill(&self, skill_name: &str) -> LibreFangResult<usize> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "DELETE FROM skill_versions WHERE skill_name = ?1",
            [skill_name],
        )
        .map_err(|e| {
            LibreFangError::memory_msg(format!("skill version cascade delete failed: {e}"))
        })
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

    /// File-backed, multi-connection pool so threads genuinely contend for the
    /// SQLite write lock — an in-memory `:memory:` DB pinned to `max_size(1)`
    /// cannot exercise the race the `Immediate` transaction exists for.
    /// Pragmas mirror production (`WAL`), or commit-time lock promotion returns
    /// `SQLITE_BUSY` without the busy handler ever running.
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
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "name = \"v1\"", "prompt v1", "dashboard", "create")
            .unwrap();
        store
            .record_version("alpha", "name = \"v2\"", "prompt v2", "dave", "evolve")
            .unwrap();

        let versions = store.list_for_skill("alpha", 10).unwrap();
        assert_eq!(versions.len(), 2);
        // Newest first.
        assert_eq!(versions[0].manifest_toml, "name = \"v2\"");
        assert_eq!(versions[0].prompt_context, "prompt v2");
        assert_eq!(versions[0].author, "dave");
        assert_eq!(versions[0].change_source, "evolve");
        assert_eq!(versions[1].manifest_toml, "name = \"v1\"");
        assert_eq!(versions[1].prompt_context, "prompt v1");
    }

    #[test]
    fn deduplicates_identical_consecutive_writes() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "same", "same prompt", "dashboard", "evolve")
            .unwrap();
        store
            .record_version("alpha", "same", "same prompt", "dave", "evolve")
            .unwrap();

        // Author is *not* part of the dedupe key: the content and the kind of
        // change are what define the event.
        let versions = store.list_for_skill("alpha", 10).unwrap();
        assert_eq!(versions.len(), 1);
    }

    /// Event-only sources pin an event, not a content change: their snapshot
    /// is identical by definition, so consecutive rows must both land rather
    /// than dedupe into one. Two `file-write` calls touching different files
    /// are two distinct history events.
    #[test]
    fn consecutive_event_only_rows_are_never_deduped() {
        let store = SkillVersionStore::new(test_pool());
        for source in ["file-write", "file-remove", "restore"] {
            let skill = format!("alpha-{source}");
            store
                .record_version(&skill, "same", "same prompt", "agent", source)
                .unwrap();
            store
                .record_version(&skill, "same", "same prompt", "agent", source)
                .unwrap();
            assert_eq!(
                store.list_for_skill(&skill, 10).unwrap().len(),
                2,
                "two consecutive `{source}` events must both be recorded"
            );
        }
    }

    /// The change_source participates in the dedupe key — re-recording the
    /// same bytes under a different source (e.g. a `restore` of the current
    /// content) is its own event and must be distinguishable in the history.
    #[test]
    fn same_content_under_a_different_source_is_recorded() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "same", "same prompt", "dashboard", "evolve")
            .unwrap();
        store
            .record_version("alpha", "same", "same prompt", "dashboard", "restore")
            .unwrap();

        let versions = store.list_for_skill("alpha", 10).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].change_source, "restore");
    }

    /// A prompt-only change is a real change even when the manifest TOML is
    /// byte-identical.
    #[test]
    fn prompt_only_change_is_recorded() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "same toml", "prompt v1", "dashboard", "evolve")
            .unwrap();
        store
            .record_version("alpha", "same toml", "prompt v2", "dashboard", "evolve")
            .unwrap();

        let versions = store.list_for_skill("alpha", 10).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].prompt_context, "prompt v2");
    }

    #[test]
    fn trims_to_cap() {
        let store = SkillVersionStore::new(test_pool());
        for i in 0..(MAX_VERSIONS_PER_SKILL + 10) {
            store
                .record_version(
                    "alpha",
                    &format!("iteration = {i}"),
                    "prompt",
                    "test",
                    "evolve",
                )
                .unwrap();
        }
        let versions = store.list_for_skill("alpha", 200).unwrap();
        assert_eq!(versions.len(), MAX_VERSIONS_PER_SKILL);
        // The newest snapshot survives the trim.
        assert_eq!(
            versions[0].manifest_toml,
            format!("iteration = {}", MAX_VERSIONS_PER_SKILL + 9)
        );
    }

    /// Event rows have their own budget: a burst of supporting-file events
    /// must not evict the content snapshots (each carrying a full manifest +
    /// prompt copy) those events happened at.
    #[test]
    fn event_rows_do_not_evict_content_versions() {
        let store = SkillVersionStore::new(test_pool());
        for i in 0..10 {
            store
                .record_version(
                    "alpha",
                    &format!("iteration = {i}"),
                    &format!("prompt {i}"),
                    "dashboard",
                    "evolve",
                )
                .unwrap();
        }
        for _ in 0..60 {
            store
                .record_version(
                    "alpha",
                    "iteration = 9",
                    "prompt 9",
                    "agent",
                    SkillWrite::FileWrite.as_str(),
                )
                .unwrap();
        }

        let versions = store.list_for_skill("alpha", 200).unwrap();
        let content = versions
            .iter()
            .filter(|row| row.change_source == "evolve")
            .count();
        let events = versions
            .iter()
            .filter(|row| row.change_source == "file-write")
            .count();
        assert_eq!(
            content, 10,
            "all content versions must survive: {versions:?}"
        );
        assert_eq!(events, MAX_EVENT_ROWS, "event rows stay capped");
        // The newest content snapshot is still there, restore-able.
        let newest_content = versions
            .iter()
            .find(|row| row.change_source == "evolve")
            .expect("a content row must remain");
        assert_eq!(newest_content.manifest_toml, "iteration = 9");
    }

    #[test]
    fn removal_recorded_after_only_counts_later_deletes() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "v1", "p1", "dashboard", "create")
            .unwrap();
        let create_id = store.list_for_skill("alpha", 10).unwrap()[0].id;
        store
            .record_version("alpha", "v2", "p2", "dashboard", "evolve")
            .unwrap();
        let evolve_id = store.list_for_skill("alpha", 10).unwrap()[0].id;

        assert!(!store.removal_recorded_after("alpha", create_id).unwrap());
        store
            .record_version(
                "alpha",
                "v2",
                "p2",
                "dashboard",
                SkillWrite::Delete.as_str(),
            )
            .unwrap();
        assert!(store.removal_recorded_after("alpha", create_id).unwrap());
        assert!(store.removal_recorded_after("alpha", evolve_id).unwrap());

        // A later re-install starts a new generation of versions, but the
        // delete still separates it from everything recorded before.
        store
            .record_version("alpha", "v3", "p3", "dashboard", "create")
            .unwrap();
        let reinstall_id = store.list_for_skill("alpha", 10).unwrap()[0].id;
        assert!(!store.removal_recorded_after("alpha", reinstall_id).unwrap());
        assert!(store.removal_recorded_after("alpha", create_id).unwrap());
        assert!(!store.removal_recorded_after("beta", create_id).unwrap());
    }

    /// Four threads record the same content for the same skill at once.
    /// The `Immediate` transaction serialises them, so exactly one row lands:
    /// the check-then-insert dedupe would double-insert if it ran the SELECT
    /// outside a write lock.
    #[test]
    fn concurrent_identical_writes_record_one_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillVersionStore::new(test_file_pool(&tmp.path().join("skills.db")));

        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let store = store.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store.record_version("alpha", "same", "same prompt", "agent", "create")
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }

        let versions = store.list_for_skill("alpha", 10).unwrap();
        assert_eq!(versions.len(), 1, "{versions:?}");
    }

    #[test]
    fn get_version_by_id() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "v1", "prompt v1", "dashboard", "create")
            .unwrap();
        let id = store.list_for_skill("alpha", 10).unwrap()[0].id;

        let row = store.get_version(id).unwrap().unwrap();
        assert_eq!(row.manifest_toml, "v1");
        assert_eq!(row.prompt_context, "prompt v1");
        assert_eq!(row.skill_name, "alpha");

        assert!(store.get_version(99999).unwrap().is_none());
    }

    /// A read failure on the dedup SELECT must surface as its own error, not
    /// get swallowed into "no previous version" and reappear as a misleading
    /// insert failure against a table that in fact exists.
    #[test]
    fn dedup_read_failure_is_reported_as_a_read_failure() {
        let store = SkillVersionStore::new(test_pool());
        // Drop the table out from under the store to force the SELECT to fail
        // with something other than "no rows".
        store
            .pool
            .get()
            .unwrap()
            .execute("DROP TABLE skill_versions", [])
            .unwrap();

        let err = store
            .record_version("alpha", "v1", "prompt", "dashboard", "create")
            .unwrap_err();
        assert!(
            err.to_string().contains("dedup read failed"),
            "expected the dedup read's own error, got: {err}"
        );
    }

    #[test]
    fn delete_cascade() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "v1", "p1", "dashboard", "create")
            .unwrap();
        store
            .record_version("alpha", "v2", "p2", "dashboard", "evolve")
            .unwrap();
        let deleted = store.delete_for_skill("alpha").unwrap();
        assert_eq!(deleted, 2);
        assert!(store.list_for_skill("alpha", 10).unwrap().is_empty());
    }

    #[test]
    fn separate_skills_are_independent() {
        let store = SkillVersionStore::new(test_pool());
        store
            .record_version("alpha", "a1", "pa", "dashboard", "create")
            .unwrap();
        store
            .record_version("beta", "b1", "pb", "dashboard", "create")
            .unwrap();

        assert_eq!(store.list_for_skill("alpha", 10).unwrap().len(), 1);
        assert_eq!(store.list_for_skill("beta", 10).unwrap().len(), 1);

        store.delete_for_skill("alpha").unwrap();
        assert!(store.list_for_skill("alpha", 10).unwrap().is_empty());
        assert_eq!(store.list_for_skill("beta", 10).unwrap().len(), 1);
    }
}
