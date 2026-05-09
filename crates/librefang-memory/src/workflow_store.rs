//! SQLite-backed workflow run store.
//!
//! Persists workflow runs so they survive daemon restarts and power loss.
//! Unlike the previous JSON file approach, Running and Pending states are
//! now durable — the daemon can resume or recover them on boot.
//!
//! The store is a thin CRUD layer; serialisation between the kernel's
//! `WorkflowRun` and `WorkflowRunRow` happens in the kernel, not here.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

/// A flat row corresponding to the `workflow_runs` SQLite table.
///
/// All fields map directly to table columns. Complex nested data
/// (step_results, paused_variables) is stored as JSON text — the
/// store does not interpret these; the kernel serializes/deserializes
/// them.
#[derive(Debug, Clone)]
pub struct WorkflowRunRow {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub state: String,
    pub input: String,
    pub output: Option<String>,
    pub error: Option<String>,
    pub resume_token: Option<String>,
    pub pause_reason: Option<String>,
    pub paused_at: Option<String>,
    pub paused_step_index: Option<i64>,
    pub paused_variables: Option<String>,
    pub paused_current_input: Option<String>,
    pub step_results: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub created_at: String,
}

/// Persistent workflow run store backed by SQLite.
///
/// Shares the same r2d2 connection pool as every other store in
/// `MemorySubstrate`. The `workflow_runs` table is created by
/// `migration::migrate_v37`, which runs before this store is constructed.
#[derive(Clone)]
pub struct WorkflowStore {
    pool: Pool<SqliteConnectionManager>,
}

impl WorkflowStore {
    /// Wrap an existing connection pool.
    ///
    /// The caller must ensure `migration::run_migrations` has already
    /// executed so the `workflow_runs` table exists.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Insert or update a workflow run row.
    ///
    /// Uses `ON CONFLICT DO UPDATE` (not `INSERT OR REPLACE`) to avoid
    /// the implicit DELETE+INSERT that would reset ROWID and break
    /// future foreign keys. When the run reaches a terminal state
    /// (Completed / Failed / Paused), the caller should follow up with
    /// [`Self::wal_checkpoint`] to flush the WAL.
    pub fn upsert_run(&self, row: &WorkflowRunRow) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute(
            "INSERT INTO workflow_runs (
                id, workflow_id, workflow_name, state, input, output, error,
                resume_token, pause_reason, paused_at, paused_step_index,
                paused_variables, paused_current_input,
                step_results, started_at, completed_at, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11,
                ?12, ?13,
                ?14, ?15, ?16, ?17
            ) ON CONFLICT(id) DO UPDATE SET
                state = excluded.state,
                input = excluded.input,
                output = excluded.output,
                error = excluded.error,
                resume_token = excluded.resume_token,
                pause_reason = excluded.pause_reason,
                paused_at = excluded.paused_at,
                paused_step_index = excluded.paused_step_index,
                paused_variables = excluded.paused_variables,
                paused_current_input = excluded.paused_current_input,
                step_results = excluded.step_results,
                completed_at = excluded.completed_at",
            rusqlite::params![
                row.id,
                row.workflow_id,
                row.workflow_name,
                row.state,
                row.input,
                row.output,
                row.error,
                row.resume_token,
                row.pause_reason,
                row.paused_at,
                row.paused_step_index,
                row.paused_variables,
                row.paused_current_input,
                row.step_results,
                row.started_at,
                row.completed_at,
                row.created_at,
            ],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("workflow upsert failed: {e}")))?;
        Ok(())
    }

    /// Get a single workflow run by ID.
    pub fn get_run(&self, id: &str) -> LibreFangResult<Option<WorkflowRunRow>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = c
            .prepare(
                "SELECT id, workflow_id, workflow_name, state, input, output, error,
                        resume_token, pause_reason, paused_at, paused_step_index,
                        paused_variables, paused_current_input,
                        step_results, started_at, completed_at, created_at
                 FROM workflow_runs WHERE id = ?1",
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("workflow get_run prepare failed: {e}"))
            })?;
        let mut rows = stmt
            .query_map(rusqlite::params![id], row_from_sqlite)
            .map_err(|e| {
                LibreFangError::memory_msg(format!("workflow get_run query failed: {e}"))
            })?;
        match rows.next() {
            Some(Ok(row)) => Ok(Some(row)),
            Some(Err(e)) => Err(LibreFangError::memory_msg(format!(
                "workflow get_run row read failed: {e}"
            ))),
            None => Ok(None),
        }
    }

    /// List workflow runs, optionally filtered by state.
    pub fn list_runs(&self, state_filter: Option<&str>) -> LibreFangResult<Vec<WorkflowRunRow>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match state_filter {
            Some(state) => (
                "SELECT id, workflow_id, workflow_name, state, input, output, error,
                        resume_token, pause_reason, paused_at, paused_step_index,
                        paused_variables, paused_current_input,
                        step_results, started_at, completed_at, created_at
                 FROM workflow_runs WHERE state = ?1 ORDER BY started_at DESC"
                    .to_string(),
                vec![Box::new(state.to_string()) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                "SELECT id, workflow_id, workflow_name, state, input, output, error,
                        resume_token, pause_reason, paused_at, paused_step_index,
                        paused_variables, paused_current_input,
                        step_results, started_at, completed_at, created_at
                 FROM workflow_runs ORDER BY started_at DESC"
                    .to_string(),
                vec![],
            ),
        };
        let mut stmt = c.prepare(&sql).map_err(|e| {
            LibreFangError::memory_msg(format!("workflow list_runs prepare failed: {e}"))
        })?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), row_from_sqlite)
            .map_err(|e| {
                LibreFangError::memory_msg(format!("workflow list_runs query failed: {e}"))
            })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| {
                LibreFangError::memory_msg(format!("workflow list_runs row read failed: {e}"))
            })?);
        }
        Ok(result)
    }

    /// Delete a workflow run by ID. Returns true if a row was deleted.
    pub fn delete_run(&self, id: &str) -> LibreFangResult<bool> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let affected = c
            .execute(
                "DELETE FROM workflow_runs WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(|e| LibreFangError::memory_msg(format!("workflow delete_run failed: {e}")))?;
        Ok(affected > 0)
    }

    /// Load all workflow runs from the database. Used at boot time to
    /// populate the in-memory DashMap.
    pub fn load_all_runs(&self) -> LibreFangResult<Vec<WorkflowRunRow>> {
        self.list_runs(None)
    }

    /// Count workflow runs. Used by the JSON-to-SQLite migration to
    /// check whether the table is empty.
    pub fn count_runs(&self) -> LibreFangResult<usize> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))
            .map_err(|e| LibreFangError::memory_msg(format!("workflow count_runs failed: {e}")))?;
        Ok(count as usize)
    }

    /// Bulk-insert rows inside a single transaction (all-or-nothing).
    /// Used by the JSON-to-SQLite migration so a crash mid-import
    /// leaves the table empty rather than partially populated.
    pub fn bulk_upsert_runs(&self, rows: &[WorkflowRunRow]) -> LibreFangResult<usize> {
        let mut c = self.pool.get().map_err(LibreFangError::memory)?;
        let tx = c.transaction().map_err(|e| {
            LibreFangError::memory_msg(format!("workflow bulk_upsert begin failed: {e}"))
        })?;
        let mut count = 0usize;
        for row in rows {
            tx.execute(
                "INSERT INTO workflow_runs (
                    id, workflow_id, workflow_name, state, input, output, error,
                    resume_token, pause_reason, paused_at, paused_step_index,
                    paused_variables, paused_current_input,
                    step_results, started_at, completed_at, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10, ?11,
                    ?12, ?13,
                    ?14, ?15, ?16, ?17
                ) ON CONFLICT(id) DO UPDATE SET
                    state = excluded.state,
                    output = excluded.output,
                    error = excluded.error,
                    step_results = excluded.step_results,
                    completed_at = excluded.completed_at",
                rusqlite::params![
                    row.id,
                    row.workflow_id,
                    row.workflow_name,
                    row.state,
                    row.input,
                    row.output,
                    row.error,
                    row.resume_token,
                    row.pause_reason,
                    row.paused_at,
                    row.paused_step_index,
                    row.paused_variables,
                    row.paused_current_input,
                    row.step_results,
                    row.started_at,
                    row.completed_at,
                    row.created_at,
                ],
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("workflow bulk_upsert row failed: {e}"))
            })?;
            count += 1;
        }
        tx.commit().map_err(|e| {
            LibreFangError::memory_msg(format!("workflow bulk_upsert commit failed: {e}"))
        })?;
        Ok(count)
    }

    /// Force a WAL checkpoint to flush writes to the main database file.
    ///
    /// Called after upserting terminal-state runs (Completed / Failed /
    /// Paused) to ensure those state transitions are durable even if the
    /// daemon crashes before the next automatic checkpoint. Uses PASSIVE
    /// mode so it never blocks concurrent readers.
    pub fn wal_checkpoint(&self) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .map_err(|e| {
                LibreFangError::memory_msg(format!("workflow wal_checkpoint failed: {e}"))
            })?;
        Ok(())
    }
}

/// Map a SQLite row to a `WorkflowRunRow`.
fn row_from_sqlite(row: &rusqlite::Row<'_>) -> Result<WorkflowRunRow, rusqlite::Error> {
    Ok(WorkflowRunRow {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        workflow_name: row.get(2)?,
        state: row.get(3)?,
        input: row.get(4)?,
        output: row.get(5)?,
        error: row.get(6)?,
        resume_token: row.get(7)?,
        pause_reason: row.get(8)?,
        paused_at: row.get(9)?,
        paused_step_index: row.get(10)?,
        paused_variables: row.get(11)?,
        paused_current_input: row.get(12)?,
        step_results: row.get(13)?,
        started_at: row.get(14)?,
        completed_at: row.get(15)?,
        created_at: row.get(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_store() -> WorkflowStore {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        crate::migration::run_migrations(&pool.get().unwrap()).expect("migrations must apply");
        WorkflowStore::new(pool)
    }

    fn sample_row(id: &str, state: &str) -> WorkflowRunRow {
        WorkflowRunRow {
            id: id.to_string(),
            workflow_id: "wf-001".to_string(),
            workflow_name: "test-workflow".to_string(),
            state: state.to_string(),
            input: "hello world".to_string(),
            output: None,
            error: None,
            resume_token: None,
            pause_reason: None,
            paused_at: None,
            paused_step_index: None,
            paused_variables: None,
            paused_current_input: None,
            step_results: "[]".to_string(),
            started_at: "2026-05-06T00:00:00Z".to_string(),
            completed_at: None,
            created_at: "2026-05-06T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn upsert_and_get() {
        let store = in_memory_store();
        let row = sample_row("run-1", "running");
        store.upsert_run(&row).unwrap();

        let loaded = store.get_run("run-1").unwrap().expect("row must exist");
        assert_eq!(loaded.id, "run-1");
        assert_eq!(loaded.state, "running");
        assert_eq!(loaded.workflow_name, "test-workflow");
        assert_eq!(loaded.input, "hello world");
    }

    #[test]
    fn upsert_replaces_existing() {
        let store = in_memory_store();
        let mut row = sample_row("run-1", "running");
        store.upsert_run(&row).unwrap();

        row.state = "completed".to_string();
        row.output = Some("done".to_string());
        store.upsert_run(&row).unwrap();

        let loaded = store.get_run("run-1").unwrap().unwrap();
        assert_eq!(loaded.state, "completed");
        assert_eq!(loaded.output, Some("done".to_string()));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let store = in_memory_store();
        assert!(store.get_run("no-such-id").unwrap().is_none());
    }

    #[test]
    fn list_runs_unfiltered() {
        let store = in_memory_store();
        store.upsert_run(&sample_row("r1", "running")).unwrap();
        store.upsert_run(&sample_row("r2", "completed")).unwrap();
        store.upsert_run(&sample_row("r3", "failed")).unwrap();

        let all = store.list_runs(None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn list_runs_filtered_by_state() {
        let store = in_memory_store();
        store.upsert_run(&sample_row("r1", "running")).unwrap();
        store.upsert_run(&sample_row("r2", "completed")).unwrap();
        store.upsert_run(&sample_row("r3", "completed")).unwrap();

        let completed = store.list_runs(Some("completed")).unwrap();
        assert_eq!(completed.len(), 2);
        assert!(completed.iter().all(|r| r.state == "completed"));

        let running = store.list_runs(Some("running")).unwrap();
        assert_eq!(running.len(), 1);
    }

    #[test]
    fn delete_run() {
        let store = in_memory_store();
        store.upsert_run(&sample_row("r1", "running")).unwrap();
        assert!(store.delete_run("r1").unwrap());
        assert!(!store.delete_run("r1").unwrap()); // already gone
        assert!(store.get_run("r1").unwrap().is_none());
    }

    #[test]
    fn count_runs() {
        let store = in_memory_store();
        assert_eq!(store.count_runs().unwrap(), 0);
        store.upsert_run(&sample_row("r1", "running")).unwrap();
        store.upsert_run(&sample_row("r2", "failed")).unwrap();
        assert_eq!(store.count_runs().unwrap(), 2);
    }

    #[test]
    fn load_all_runs() {
        let store = in_memory_store();
        store.upsert_run(&sample_row("r1", "pending")).unwrap();
        store.upsert_run(&sample_row("r2", "running")).unwrap();
        let all = store.load_all_runs().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn paused_state_round_trip() {
        let store = in_memory_store();
        let mut row = sample_row("r1", "paused");
        row.resume_token = Some("tok-abc".to_string());
        row.pause_reason = Some("approval needed".to_string());
        row.paused_at = Some("2026-05-06T01:00:00Z".to_string());
        row.paused_step_index = Some(2);
        row.paused_variables = Some(r#"{"x":"1","y":"2"}"#.to_string());
        row.paused_current_input = Some("step-2-output".to_string());
        store.upsert_run(&row).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.resume_token, Some("tok-abc".to_string()));
        assert_eq!(loaded.pause_reason, Some("approval needed".to_string()));
        assert_eq!(loaded.paused_step_index, Some(2));
        assert_eq!(
            loaded.paused_variables,
            Some(r#"{"x":"1","y":"2"}"#.to_string())
        );
        assert_eq!(
            loaded.paused_current_input,
            Some("step-2-output".to_string())
        );
    }

    #[test]
    fn empty_paused_variables_round_trip() {
        let store = in_memory_store();
        let mut row = sample_row("r1", "paused");
        row.resume_token = Some("tok-abc".to_string());
        row.pause_reason = Some("waiting".to_string());
        row.paused_at = Some("2026-05-06T01:00:00Z".to_string());
        // paused_variables is None (empty) — must survive the round trip
        row.paused_variables = None;
        store.upsert_run(&row).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.paused_variables, None);
    }

    #[test]
    fn wal_checkpoint_does_not_error() {
        let store = in_memory_store();
        // In-memory databases do not use WAL, but the PRAGMA should still
        // succeed without error.
        store.wal_checkpoint().unwrap();
    }

    #[test]
    fn invalid_state_rejected_by_check_constraint() {
        let store = in_memory_store();
        let row = sample_row("r1", "invalid_state");
        let result = store.upsert_run(&row);
        assert!(
            result.is_err(),
            "CHECK constraint must reject invalid state"
        );
    }

    #[test]
    fn started_at_and_created_at_are_distinct() {
        let store = in_memory_store();
        let mut row = sample_row("r1", "running");
        row.started_at = "2026-05-06T01:00:00Z".to_string();
        row.created_at = "2026-05-06T00:30:00Z".to_string();
        store.upsert_run(&row).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.started_at, "2026-05-06T01:00:00Z");
        assert_eq!(loaded.created_at, "2026-05-06T00:30:00Z");
    }

    #[test]
    fn bulk_upsert_runs_all_or_nothing() {
        let store = in_memory_store();
        let rows = vec![
            sample_row("r1", "completed"),
            sample_row("r2", "failed"),
            sample_row("r3", "paused"),
        ];
        let count = store.bulk_upsert_runs(&rows).unwrap();
        assert_eq!(count, 3);
        assert_eq!(store.count_runs().unwrap(), 3);
    }

    #[test]
    fn bulk_upsert_rejects_invalid_state() {
        let store = in_memory_store();
        let rows = vec![
            sample_row("r1", "completed"),
            sample_row("r2", "bogus"), // invalid — should abort the whole batch
        ];
        let result = store.bulk_upsert_runs(&rows);
        assert!(result.is_err());
        // Transaction rolled back: nothing inserted.
        assert_eq!(store.count_runs().unwrap(), 0);
    }

    #[test]
    fn on_conflict_preserves_created_at() {
        let store = in_memory_store();
        let mut row = sample_row("r1", "running");
        row.created_at = "2026-05-06T00:00:00Z".to_string();
        store.upsert_run(&row).unwrap();

        // Update with a different created_at — ON CONFLICT must NOT
        // overwrite created_at (it is excluded from the DO UPDATE SET).
        row.state = "completed".to_string();
        row.created_at = "2099-01-01T00:00:00Z".to_string();
        store.upsert_run(&row).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.state, "completed");
        assert_eq!(
            loaded.created_at, "2026-05-06T00:00:00Z",
            "created_at must be immutable after first insert"
        );
    }
}
