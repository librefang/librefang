//! SQLite-backed ephemeral worker run records (refs #7752).
//!
//! An ephemeral worker (`agent_spawn` with `ephemeral: true`, engine in `librefang-kernel`'s `ephemeral_spawn`) runs one turn and vanishes: no registry entry, no persisted session, and a mission workspace that is deleted when the run ends.
//! That left an operator with nothing to inspect — spend was billed to the parent through `usage_events.billed_agent_id`, but the *work* that produced the spend was invisible, so "this agent fired twelve workers this turn" could not be answered at all.
//!
//! This table is the run record that closes that gap, and it is deliberately **not** a session.
//!
//! ## Why a dedicated table rather than a child session row
//!
//! The obvious-looking fix — clear the worker's `incognito` flag so the agent loop persists its session under the parent's `AgentId` — is wrong, and the reasons are worth writing down because the flag is one line and the damage is not.
//! `incognito` gates far more than the session write: it also gates the episodic-memory write, the context-engine `after_turn` advance, and the proactive-memory `auto_memorize` pass, all of which key on `session.agent_id` — which for a worker *is* the parent.
//! Clearing it would fold a delegated sub-run's exchange into the parent's own recall, teaching the parent it said things it never said, and would file the worker's throwaway session among the parent's real conversations in every session-list view.
//!
//! A run record is the narrower and correct artefact: it is written once, by the kernel, after the run, out of band from the agent loop's persistence boundary, and it records what was delegated, what came back, and what it cost without touching the parent's memory or conversation history.
//!
//! ## Retention
//!
//! Two mechanisms, both by construction rather than by sweeper:
//!
//! - **Cascade.** `parent_agent_id` rows are deleted with the parent agent, through [`execute_ephemeral_run_agent_deletes`] wired into the same transaction as the session and structured-store cascades (`MemorySubstrate::remove_agent`).
//! - **Cap.** The ephemeral path is designed to be called cheaply and often, so an unbounded per-parent history would recreate a milder version of the row-bloat problem this feature exists to fix.
//!   Each insert trims its own parent back to [`MAX_RUNS_PER_PARENT`] most recent rows inside the same transaction.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

/// Run outcomes the `status` column accepts, matching the table's `CHECK` constraint.
pub const VALID_STATUSES: &[&str] = &["completed", "failed"];

/// Most recent run records kept per parent agent.
///
/// Trimmed on insert, so the table's size is bounded by (number of agents × this) regardless of how often the spawn path is called.
/// Deliberately a compiled constant rather than a config knob: the point of the cap is that no operator has to know it exists for the table to stay bounded.
pub const MAX_RUNS_PER_PARENT: usize = 200;

/// Character ceiling applied to the delegated task and the worker's answer before they are stored.
///
/// The record has to be inspectable to be worth writing — "what was delegated, what came back" is the operator question — but a worker's answer is model output of unbounded length, and this table takes one row per spawn.
/// Truncation is marked in the stored value so a reader can tell a short answer from a clipped one.
pub const MAX_TEXT_LEN: usize = 4096;

/// Suffix appended to a value clipped at [`MAX_TEXT_LEN`].
const TRUNCATION_MARKER: &str = "… [truncated]";

/// One completed or failed ephemeral worker run, attributed to the agent that spawned it.
#[derive(Debug, Clone)]
pub struct EphemeralRunRow {
    /// Row identity. Unrelated to any `AgentId` or `SessionId` — a worker has neither.
    pub id: String,
    /// The agent this run was performed for, billed to, and authorized as.
    ///
    /// Matches `usage_events.billed_agent_id` for the same run, so the ledger and this table agree on the owner.
    pub parent_agent_id: String,
    /// Caller-supplied mission label, before sanitization.
    pub label: String,
    /// Uid-style display name the worker ran under (`<label>-<8 hex>`), also the name its mission workspace directory had.
    pub worker_name: String,
    /// Agent type whose template supplied the worker's persona, when one was named.
    pub agent_type: Option<String>,
    /// The task the worker was given, truncated to [`MAX_TEXT_LEN`].
    pub task: String,
    /// The worker's final answer, truncated to [`MAX_TEXT_LEN`]. Empty for a failed run.
    pub response: String,
    /// One of [`VALID_STATUSES`].
    pub status: String,
    /// Why the run failed, when it did.
    pub error: Option<String>,
    /// Provider actually used for the run.
    pub provider: String,
    /// Model actually used for the run.
    pub model: String,
    /// Loop iterations consumed.
    pub iterations: i64,
    /// Tool calls the worker issued.
    pub tool_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Cost billed to the parent, in USD.
    pub cost_usd: f64,
    /// Wall-clock duration of the run.
    pub latency_ms: i64,
    /// RFC 3339 start timestamp.
    pub started_at: String,
    /// RFC 3339 finish timestamp.
    pub finished_at: String,
}

/// Clip `text` to [`MAX_TEXT_LEN`] characters, marking the clip.
///
/// Counts `char`s and slices on a boundary the iterator produced, so a multi-byte grapheme is never split mid-codepoint — `&text[..MAX_TEXT_LEN]` on a byte index would panic on any non-ASCII answer.
#[must_use]
pub fn truncate_for_record(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT_LEN) {
        None => text.to_string(),
        Some((byte_idx, _)) => format!("{}{TRUNCATION_MARKER}", &text[..byte_idx]),
    }
}

/// Persistent ephemeral-run store backed by SQLite.
///
/// Shares the connection pool every other store in `MemorySubstrate` uses. The `ephemeral_runs` table is created by `migration::migrate_v51`, which runs before this store is constructed.
#[derive(Clone)]
pub struct EphemeralRunStore {
    pool: Pool<SqliteConnectionManager>,
}

impl EphemeralRunStore {
    /// Wrap an existing connection pool.
    ///
    /// The caller must ensure `migration::run_migrations` has already executed so the `ephemeral_runs` table exists.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Record one finished ephemeral run and trim its parent back to [`MAX_RUNS_PER_PARENT`].
    ///
    /// Insert and trim share one transaction: a crash between them would otherwise leave the cap enforced against a row set that no longer includes the row just written.
    pub fn record_run(&self, row: &EphemeralRunRow) -> LibreFangResult<()> {
        if !VALID_STATUSES.contains(&row.status.as_str()) {
            return Err(LibreFangError::memory_msg(format!(
                "invalid ephemeral run status '{}', expected one of {}",
                row.status,
                VALID_STATUSES.join(", ")
            )));
        }
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(LibreFangError::memory)?;
        tx.execute(
            "INSERT INTO ephemeral_runs (
                id, parent_agent_id, label, worker_name, agent_type,
                task, response, status, error, provider, model,
                iterations, tool_calls, input_tokens, output_tokens,
                cost_usd, latency_ms, started_at, finished_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19
            )",
            rusqlite::params![
                row.id,
                row.parent_agent_id,
                row.label,
                row.worker_name,
                row.agent_type,
                row.task,
                row.response,
                row.status,
                row.error,
                row.provider,
                row.model,
                row.iterations,
                row.tool_calls,
                row.input_tokens,
                row.output_tokens,
                row.cost_usd,
                row.latency_ms,
                row.started_at,
                row.finished_at,
            ],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("ephemeral run insert failed: {e}")))?;

        // Ordered by `finished_at DESC, rowid DESC` so runs that finished inside the same clock tick still have a total order and the trim cannot pick an arbitrary one of them to keep.
        tx.execute(
            "DELETE FROM ephemeral_runs
             WHERE parent_agent_id = ?1
               AND rowid NOT IN (
                   SELECT rowid FROM ephemeral_runs
                   WHERE parent_agent_id = ?1
                   ORDER BY finished_at DESC, rowid DESC
                   LIMIT ?2
               )",
            rusqlite::params![row.parent_agent_id, MAX_RUNS_PER_PARENT as i64],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("ephemeral run trim failed: {e}")))?;

        tx.commit().map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// The most recent runs a parent agent spawned, newest first.
    ///
    /// An agent that has spawned nothing yields an empty vector, not an error — "no workers" is an ordinary answer, and the caller has no way to distinguish an agent that never spawned from one whose runs were trimmed.
    pub fn list_for_parent(
        &self,
        parent_agent_id: &str,
        limit: usize,
    ) -> LibreFangResult<Vec<EphemeralRunRow>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, parent_agent_id, label, worker_name, agent_type,
                        task, response, status, error, provider, model,
                        iterations, tool_calls, input_tokens, output_tokens,
                        cost_usd, latency_ms, started_at, finished_at
                 FROM ephemeral_runs
                 WHERE parent_agent_id = ?1
                 ORDER BY finished_at DESC, rowid DESC
                 LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(
                rusqlite::params![parent_agent_id, limit as i64],
                map_run_row,
            )
            .map_err(LibreFangError::memory)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(LibreFangError::memory)?;
        Ok(rows)
    }

    /// Total spend and run count a parent has accrued through ephemeral workers.
    ///
    /// Reads this table rather than `usage_events` on purpose: `usage_events` rolls a worker's spend up into the parent's own line and cannot separate the two back out, which is the exact question an operator asks here.
    /// Bounded by [`MAX_RUNS_PER_PARENT`], so this is a rollup of retained runs, not of all time.
    pub fn rollup_for_parent(&self, parent_agent_id: &str) -> LibreFangResult<EphemeralRunRollup> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.query_row(
            "SELECT COUNT(*), IFNULL(SUM(cost_usd), 0.0),
                    IFNULL(SUM(input_tokens), 0), IFNULL(SUM(output_tokens), 0),
                    IFNULL(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0)
             FROM ephemeral_runs WHERE parent_agent_id = ?1",
            rusqlite::params![parent_agent_id],
            |r| {
                Ok(EphemeralRunRollup {
                    runs: r.get::<_, i64>(0)? as u64,
                    cost_usd: r.get(1)?,
                    input_tokens: r.get::<_, i64>(2)? as u64,
                    output_tokens: r.get::<_, i64>(3)? as u64,
                    failed: r.get::<_, i64>(4)? as u64,
                })
            },
        )
        .map_err(LibreFangError::memory)
    }

    /// Delete every run record owned by one agent.
    ///
    /// Standalone counterpart to the cascade in `MemorySubstrate::remove_agent`, for callers holding only this store.
    pub fn delete_for_agent(&self, parent_agent_id: &str) -> LibreFangResult<usize> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let n = conn
            .execute(
                "DELETE FROM ephemeral_runs WHERE parent_agent_id = ?1",
                rusqlite::params![parent_agent_id],
            )
            .map_err(LibreFangError::memory)?;
        Ok(n)
    }
}

/// Aggregate of the run records retained for one parent agent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EphemeralRunRollup {
    pub runs: u64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub failed: u64,
}

fn map_run_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EphemeralRunRow> {
    Ok(EphemeralRunRow {
        id: r.get(0)?,
        parent_agent_id: r.get(1)?,
        label: r.get(2)?,
        worker_name: r.get(3)?,
        agent_type: r.get(4)?,
        task: r.get(5)?,
        response: r.get(6)?,
        status: r.get(7)?,
        error: r.get(8)?,
        provider: r.get(9)?,
        model: r.get(10)?,
        iterations: r.get(11)?,
        tool_calls: r.get(12)?,
        input_tokens: r.get(13)?,
        output_tokens: r.get(14)?,
        cost_usd: r.get(15)?,
        latency_ms: r.get(16)?,
        started_at: r.get(17)?,
        finished_at: r.get(18)?,
    })
}

/// Run the ephemeral-run DELETE for an agent inside the caller's transaction.
///
/// Shared by [`MemorySubstrate::remove_agent`](crate::substrate::MemorySubstrate::remove_agent) and its async sibling so a deleted agent's workers go with it in the same transaction that removes its sessions — "retention follows the parent, no orphans by construction" is the whole claim this table makes, and it is the easiest one to implement halfway.
pub(crate) fn execute_ephemeral_run_agent_deletes(
    tx: &rusqlite::Transaction<'_>,
    agent_id: &str,
) -> LibreFangResult<()> {
    tx.execute(
        "DELETE FROM ephemeral_runs WHERE parent_agent_id = ?1",
        rusqlite::params![agent_id],
    )
    .map_err(LibreFangError::memory)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> EphemeralRunStore {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        EphemeralRunStore::new(pool)
    }

    fn row(parent: &str, label: &str, finished_at: &str) -> EphemeralRunRow {
        EphemeralRunRow {
            id: uuid::Uuid::new_v4().to_string(),
            parent_agent_id: parent.to_string(),
            label: label.to_string(),
            worker_name: format!("{label}-0011aabb"),
            agent_type: None,
            task: "find the thing".to_string(),
            response: "found it".to_string(),
            status: "completed".to_string(),
            error: None,
            provider: "anthropic".to_string(),
            model: "test-model".to_string(),
            iterations: 2,
            tool_calls: 1,
            input_tokens: 100,
            output_tokens: 20,
            cost_usd: 0.25,
            latency_ms: 1234,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            finished_at: finished_at.to_string(),
        }
    }

    #[test]
    fn a_recorded_run_is_listed_under_its_parent() {
        let store = setup();
        let parent = uuid::Uuid::new_v4().to_string();
        store
            .record_run(&row(&parent, "researcher", "2026-01-01T00:00:01Z"))
            .unwrap();

        let runs = store.list_for_parent(&parent, 50).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].label, "researcher");
        assert_eq!(runs[0].worker_name, "researcher-0011aabb");
        assert_eq!(runs[0].cost_usd, 0.25);
        assert_eq!(runs[0].status, "completed");
    }

    /// A parent that spawned nothing must not see another parent's runs, and must not error.
    #[test]
    fn a_parent_with_no_runs_sees_an_empty_list() {
        let store = setup();
        let busy = uuid::Uuid::new_v4().to_string();
        let idle = uuid::Uuid::new_v4().to_string();
        store
            .record_run(&row(&busy, "researcher", "2026-01-01T00:00:01Z"))
            .unwrap();

        assert!(store.list_for_parent(&idle, 50).unwrap().is_empty());
        assert_eq!(
            store.rollup_for_parent(&idle).unwrap(),
            EphemeralRunRollup::default()
        );
    }

    #[test]
    fn runs_are_listed_newest_first() {
        let store = setup();
        let parent = uuid::Uuid::new_v4().to_string();
        store
            .record_run(&row(&parent, "first", "2026-01-01T00:00:01Z"))
            .unwrap();
        store
            .record_run(&row(&parent, "second", "2026-01-01T00:00:02Z"))
            .unwrap();

        let runs = store.list_for_parent(&parent, 50).unwrap();
        assert_eq!(
            runs.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            vec!["second", "first"]
        );
    }

    /// The cap is what keeps a cheap, frequently-called path from re-creating the row-bloat problem the run record exists to replace.
    #[test]
    fn inserts_trim_the_parent_to_the_retention_cap() {
        let store = setup();
        let parent = uuid::Uuid::new_v4().to_string();
        for i in 0..(MAX_RUNS_PER_PARENT + 5) {
            store
                .record_run(&row(
                    &parent,
                    &format!("run{i}"),
                    &format!("2026-01-01T00:00:{:02}Z", i % 60),
                ))
                .unwrap();
        }
        let runs = store
            .list_for_parent(&parent, MAX_RUNS_PER_PARENT * 2)
            .unwrap();
        assert_eq!(runs.len(), MAX_RUNS_PER_PARENT);
    }

    /// One parent's trim must never reach another parent's rows.
    #[test]
    fn the_trim_is_scoped_to_one_parent() {
        let store = setup();
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        store
            .record_run(&row(&b, "b-run", "2026-01-01T00:00:00Z"))
            .unwrap();
        for i in 0..(MAX_RUNS_PER_PARENT + 5) {
            store
                .record_run(&row(&a, &format!("a{i}"), "2026-01-01T00:00:01Z"))
                .unwrap();
        }
        assert_eq!(store.list_for_parent(&b, 50).unwrap().len(), 1);
    }

    #[test]
    fn a_rollup_sums_cost_tokens_and_failures() {
        let store = setup();
        let parent = uuid::Uuid::new_v4().to_string();
        store
            .record_run(&row(&parent, "ok", "2026-01-01T00:00:01Z"))
            .unwrap();
        let mut bad = row(&parent, "bad", "2026-01-01T00:00:02Z");
        bad.status = "failed".to_string();
        bad.error = Some("driver exploded".to_string());
        bad.response = String::new();
        store.record_run(&bad).unwrap();

        let rollup = store.rollup_for_parent(&parent).unwrap();
        assert_eq!(rollup.runs, 2);
        assert_eq!(rollup.failed, 1);
        assert_eq!(rollup.input_tokens, 200);
        assert!((rollup.cost_usd - 0.5).abs() < f64::EPSILON);
    }

    /// An unknown status must be refused by the store rather than reaching the table's CHECK constraint as an opaque SQLite error.
    #[test]
    fn an_unknown_status_is_rejected_by_name() {
        let store = setup();
        let parent = uuid::Uuid::new_v4().to_string();
        let mut bad = row(&parent, "weird", "2026-01-01T00:00:01Z");
        bad.status = "sideways".to_string();
        let err = store.record_run(&bad).unwrap_err().to_string();
        assert!(
            err.contains("sideways"),
            "error must name the bad status: {err}"
        );
        assert!(store.list_for_parent(&parent, 50).unwrap().is_empty());
    }

    #[test]
    fn deleting_an_agent_removes_only_its_own_runs() {
        let store = setup();
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        store
            .record_run(&row(&a, "a", "2026-01-01T00:00:01Z"))
            .unwrap();
        store
            .record_run(&row(&b, "b", "2026-01-01T00:00:01Z"))
            .unwrap();

        assert_eq!(store.delete_for_agent(&a).unwrap(), 1);
        assert!(store.list_for_parent(&a, 50).unwrap().is_empty());
        assert_eq!(store.list_for_parent(&b, 50).unwrap().len(), 1);
    }

    /// A worker's answer is model output of unbounded length; the clip must land on a `char` boundary or a non-ASCII answer panics the slice.
    #[test]
    fn truncation_lands_on_a_char_boundary() {
        let short = "hello";
        assert_eq!(truncate_for_record(short), short);

        let multibyte = "日".repeat(MAX_TEXT_LEN + 100);
        let clipped = truncate_for_record(&multibyte);
        assert!(clipped.ends_with(TRUNCATION_MARKER));
        assert_eq!(
            clipped.chars().count(),
            MAX_TEXT_LEN + TRUNCATION_MARKER.chars().count()
        );
    }

    /// Exactly `MAX_TEXT_LEN` characters is not truncation — the marker would otherwise claim content was dropped when none was.
    #[test]
    fn text_at_exactly_the_cap_is_not_marked_truncated() {
        let exact = "a".repeat(MAX_TEXT_LEN);
        assert_eq!(truncate_for_record(&exact), exact);
    }
}
