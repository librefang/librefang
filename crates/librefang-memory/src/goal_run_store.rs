//! SQLite-backed goal run store (#5744 follow-up).
//!
//! Persists active goal-run state so long-horizon goal runs survive daemon
//! restarts and power loss. Without it the `GoalRunner`'s in-memory DashMap
//! was the only record of an active run, and a restart silently dropped every
//! in-flight run — unlike workflow runs, which gained SQLite durability in
//! v37. This store is the workflow-parity counterpart for goals.
//!
//! The store is a thin CRUD layer; serialisation between the kernel's
//! `GoalRunState` and `GoalRunRow` happens in the kernel, not here.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

/// A flat row corresponding to the `goal_runs` SQLite table.
///
/// All fields map directly to table columns. `goal_id` is the primary key —
/// at most one run is active per goal, so a save for a goal that already has
/// a row updates it in place.
#[derive(Debug, Clone)]
pub struct GoalRunRow {
    pub goal_id: String,
    pub agent_id: String,
    pub phase: String,
    pub iteration: i64,
    pub max_iterations: i64,
    pub last_progress: i64,
    pub last_error: Option<String>,
    pub started_at: String,
    pub updated_at: String,
}

/// Persistent goal run store backed by SQLite.
///
/// Shares the same r2d2 connection pool as every other store in
/// `MemorySubstrate`. The `goal_runs` table is created by
/// `migration::migrate_v42`, which runs before this store is constructed.
#[derive(Clone)]
pub struct GoalRunStore {
    pool: Pool<SqliteConnectionManager>,
}

impl GoalRunStore {
    /// Wrap an existing connection pool.
    ///
    /// The caller must ensure `migration::run_migrations` has already
    /// executed so the `goal_runs` table exists.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Insert or update a goal run row, keyed on `goal_id`.
    ///
    /// Uses `ON CONFLICT DO UPDATE` (not `INSERT OR REPLACE`) to avoid the
    /// implicit DELETE+INSERT that would reset ROWID. `created_at` is omitted
    /// from the INSERT column list so the schema default `datetime('now')`
    /// fires once on first insert and is preserved across later updates.
    pub fn save_run(&self, row: &GoalRunRow) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute(
            "INSERT INTO goal_runs (
                goal_id, agent_id, phase, iteration, max_iterations,
                last_progress, last_error, started_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9
            ) ON CONFLICT(goal_id) DO UPDATE SET
                agent_id = excluded.agent_id,
                phase = excluded.phase,
                iteration = excluded.iteration,
                max_iterations = excluded.max_iterations,
                last_progress = excluded.last_progress,
                last_error = excluded.last_error,
                started_at = excluded.started_at,
                updated_at = excluded.updated_at",
            rusqlite::params![
                row.goal_id,
                row.agent_id,
                row.phase,
                row.iteration,
                row.max_iterations,
                row.last_progress,
                row.last_error,
                row.started_at,
                row.updated_at,
            ],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("goal run save failed: {e}")))?;
        Ok(())
    }

    /// Get a single goal run by goal ID.
    pub fn get_run(&self, goal_id: &str) -> LibreFangResult<Option<GoalRunRow>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = c
            .prepare(
                "SELECT goal_id, agent_id, phase, iteration, max_iterations,
                        last_progress, last_error, started_at, updated_at
                 FROM goal_runs WHERE goal_id = ?1",
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("goal run get_run prepare failed: {e}"))
            })?;
        let mut rows = stmt
            .query_map(rusqlite::params![goal_id], row_from_sqlite)
            .map_err(|e| {
                LibreFangError::memory_msg(format!("goal run get_run query failed: {e}"))
            })?;
        match rows.next() {
            Some(Ok(row)) => Ok(Some(row)),
            Some(Err(e)) => Err(LibreFangError::memory_msg(format!(
                "goal run get_run row read failed: {e}"
            ))),
            None => Ok(None),
        }
    }

    /// Load all goal runs, ordered by `started_at` (newest first) so the
    /// boot-time recovery walk is deterministic across processes.
    pub fn load_all_runs(&self) -> LibreFangResult<Vec<GoalRunRow>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = c
            .prepare(
                "SELECT goal_id, agent_id, phase, iteration, max_iterations,
                        last_progress, last_error, started_at, updated_at
                 FROM goal_runs ORDER BY started_at DESC",
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("goal run load_all prepare failed: {e}"))
            })?;
        let rows = stmt.query_map([], row_from_sqlite).map_err(|e| {
            LibreFangError::memory_msg(format!("goal run load_all query failed: {e}"))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| {
                LibreFangError::memory_msg(format!("goal run load_all row read failed: {e}"))
            })?);
        }
        Ok(result)
    }

    /// Delete a goal run by goal ID. Returns true if a row was deleted.
    pub fn delete_run(&self, goal_id: &str) -> LibreFangResult<bool> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let affected = c
            .execute(
                "DELETE FROM goal_runs WHERE goal_id = ?1",
                rusqlite::params![goal_id],
            )
            .map_err(|e| LibreFangError::memory_msg(format!("goal run delete failed: {e}")))?;
        Ok(affected > 0)
    }

    /// Count goal runs. Used by tests and operator tooling.
    pub fn count_runs(&self) -> LibreFangResult<usize> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM goal_runs", [], |row| row.get(0))
            .map_err(|e| LibreFangError::memory_msg(format!("goal run count failed: {e}")))?;
        Ok(count as usize)
    }

    /// Force a WAL checkpoint to flush writes to the main database file.
    ///
    /// Called after persisting a terminal-phase run (the recovery sweep
    /// demoting a stale run) so the transition is durable even if the daemon
    /// crashes before the next automatic checkpoint. PASSIVE mode never blocks
    /// concurrent readers.
    pub fn wal_checkpoint(&self) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .map_err(|e| {
                LibreFangError::memory_msg(format!("goal run wal_checkpoint failed: {e}"))
            })?;
        Ok(())
    }
}

/// Map a SQLite row to a `GoalRunRow`.
fn row_from_sqlite(row: &rusqlite::Row<'_>) -> Result<GoalRunRow, rusqlite::Error> {
    Ok(GoalRunRow {
        goal_id: row.get(0)?,
        agent_id: row.get(1)?,
        phase: row.get(2)?,
        iteration: row.get(3)?,
        max_iterations: row.get(4)?,
        last_progress: row.get(5)?,
        last_error: row.get(6)?,
        started_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_store() -> GoalRunStore {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        crate::migration::run_migrations(&pool.get().unwrap()).expect("migrations must apply");
        GoalRunStore::new(pool)
    }

    fn sample_row(goal_id: &str, phase: &str) -> GoalRunRow {
        GoalRunRow {
            goal_id: goal_id.to_string(),
            agent_id: "agent-1".to_string(),
            phase: phase.to_string(),
            iteration: 0,
            max_iterations: 25,
            last_progress: 0,
            last_error: None,
            started_at: "2026-05-06T00:00:00Z".to_string(),
            updated_at: "2026-05-06T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn save_and_get() {
        let store = in_memory_store();
        let row = sample_row("goal-1", "running");
        store.save_run(&row).unwrap();

        let loaded = store.get_run("goal-1").unwrap().expect("row must exist");
        assert_eq!(loaded.goal_id, "goal-1");
        assert_eq!(loaded.phase, "running");
        assert_eq!(loaded.max_iterations, 25);
    }

    #[test]
    fn save_replaces_existing() {
        let store = in_memory_store();
        let mut row = sample_row("goal-1", "running");
        store.save_run(&row).unwrap();

        row.phase = "finished".to_string();
        row.iteration = 3;
        row.last_progress = 100;
        store.save_run(&row).unwrap();

        let loaded = store.get_run("goal-1").unwrap().unwrap();
        assert_eq!(loaded.phase, "finished");
        assert_eq!(loaded.iteration, 3);
        assert_eq!(loaded.last_progress, 100);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let store = in_memory_store();
        assert!(store.get_run("no-such-goal").unwrap().is_none());
    }

    #[test]
    fn load_all_runs() {
        let store = in_memory_store();
        store.save_run(&sample_row("g1", "running")).unwrap();
        store.save_run(&sample_row("g2", "finished")).unwrap();
        let all = store.load_all_runs().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn delete_run() {
        let store = in_memory_store();
        store.save_run(&sample_row("g1", "running")).unwrap();
        assert!(store.delete_run("g1").unwrap());
        assert!(!store.delete_run("g1").unwrap()); // already gone
        assert!(store.get_run("g1").unwrap().is_none());
    }

    #[test]
    fn count_runs() {
        let store = in_memory_store();
        assert_eq!(store.count_runs().unwrap(), 0);
        store.save_run(&sample_row("g1", "running")).unwrap();
        store.save_run(&sample_row("g2", "stopped")).unwrap();
        assert_eq!(store.count_runs().unwrap(), 2);
    }

    #[test]
    fn invalid_phase_rejected_by_check_constraint() {
        let store = in_memory_store();
        let row = sample_row("g1", "bogus_phase");
        assert!(
            store.save_run(&row).is_err(),
            "CHECK constraint must reject an unknown phase"
        );
    }

    #[test]
    fn last_error_round_trip() {
        let store = in_memory_store();
        let mut row = sample_row("g1", "stopped");
        row.last_error = Some("Interrupted by daemon restart".to_string());
        store.save_run(&row).unwrap();

        let loaded = store.get_run("g1").unwrap().unwrap();
        assert_eq!(
            loaded.last_error,
            Some("Interrupted by daemon restart".to_string())
        );
    }

    #[test]
    fn wal_checkpoint_does_not_error() {
        let store = in_memory_store();
        store.wal_checkpoint().unwrap();
    }
}
