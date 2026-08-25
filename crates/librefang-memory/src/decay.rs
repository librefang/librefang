//! Time-based memory decay — soft-deletes stale memories based on scope TTL.
//!
//! Scope rules:
//! - **USER**: Never decays (permanent user knowledge).
//! - **SESSION**: Decays after `session_ttl_days` of no access.
//! - **AGENT**: Decays after `agent_ttl_days` of no access.
//! - **EPISODIC**: Decays after `episodic_ttl_days` of no access (#7911).
//!
//! `episodic` is both the table default (`migration.rs`, `scope TEXT NOT NULL DEFAULT 'episodic'`) and the scope the agent loop writes one row into on every non-fork, non-incognito turn, so it is by far the highest-volume scope in a real store.
//! Before #7911 it was the only scope this sweep did not touch, which left the episodic layer with no exit: written every turn, never distilled by the consolidation engine (that engine only lowers `confidence` and merges near-verbatim duplicates — it never deletes by age), and never expired.
//!
//! Accessing a memory (via search/recall) resets the decay timer by updating
//! `accessed_at`, which is already handled by `SemanticStore::recall_with_embedding`.
//!
//! Decay performs a **soft delete** (`deleted = 1`, `deleted_at = <now>`)
//! rather than a hard `DELETE`. Other modules (consolidation, history queries)
//! rely on the `deleted` invariant; hard removal happens later in
//! [`prune_soft_deleted_memories`], scheduled by the kernel retention sweep.

use chrono::Utc;
use librefang_types::config::MemoryDecayConfig;
use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use tracing::{debug, info, warn};

/// Run time-based decay on the memories table.
///
/// Soft-deletes SESSION, AGENT and EPISODIC scope memories whose `accessed_at` is older than the configured TTL.
/// USER scope memories are never touched.
///
/// `accessed_at` is stored as RFC3339; rather than rely on lexicographic
/// string comparison (which is wrong as soon as offsets / `Z` vs `+00:00` /
/// fractional-second precision diverge), we wrap both sides in
/// `datetime(...)` so SQLite parses them as real timestamps before comparing.
///
/// A zero TTL disables expiry for that scope. Rows with missing or malformed
/// `accessed_at` values are left intact and reported for operator attention.
///
/// Returns the number of memories soft-deleted.
/// All scope updates share one transaction and are committed atomically.
pub fn run_decay(
    pool: &Pool<SqliteConnectionManager>,
    config: &MemoryDecayConfig,
) -> LibreFangResult<usize> {
    if !config.enabled {
        return Ok(0);
    }

    let mut db = pool.get().map_err(LibreFangError::memory)?;

    let now = Utc::now();
    let now_unix = now.timestamp();
    let mut total_deleted: usize = 0;
    let tx = db.transaction().map_err(LibreFangError::memory)?;

    // Decay SESSION scope memories — soft-delete only.
    total_deleted += decay_scope(
        &tx,
        "session_memory",
        "SESSION",
        config.session_ttl_days,
        now,
        now_unix,
    )?;

    // Decay AGENT scope memories — soft-delete only.
    total_deleted += decay_scope(
        &tx,
        "agent_memory",
        "AGENT",
        config.agent_ttl_days,
        now,
        now_unix,
    )?;

    // Decay EPISODIC scope memories — soft-delete only (#7911).
    total_deleted += decay_scope(
        &tx,
        "episodic",
        "EPISODIC",
        config.episodic_ttl_days,
        now,
        now_unix,
    )?;

    tx.commit().map_err(LibreFangError::memory)?;

    if total_deleted > 0 {
        info!(total_deleted, "Memory decay sweep completed");
    }

    Ok(total_deleted)
}

/// Soft-delete every non-deleted row in `scope` whose `accessed_at` is older than `ttl_days`, inside the caller's transaction.
///
/// A zero `ttl_days` disables expiry for that scope and is a no-op.
/// `label` is the operator-facing scope name used in log fields; it is deliberately distinct from `scope` because the log vocabulary (`SESSION` / `AGENT` / `EPISODIC`) predates the stored values (`session_memory` / `agent_memory` / `episodic`) and operator runbooks grep for the former.
///
/// `accessed_at` is stored as RFC3339; both sides are wrapped in `datetime(...)` so SQLite parses them as real timestamps rather than comparing strings whose offsets or fractional-second precision may differ.
/// Rows with a missing or malformed `accessed_at` are left intact and counted into a single warn line so an operator can find them.
fn decay_scope(
    tx: &rusqlite::Transaction<'_>,
    scope: &str,
    label: &str,
    ttl_days: u32,
    now: chrono::DateTime<Utc>,
    now_unix: i64,
) -> LibreFangResult<usize> {
    if ttl_days == 0 {
        return Ok(0);
    }
    let cutoff = now - chrono::Duration::days(i64::from(ttl_days));
    let cutoff_str = cutoff.to_rfc3339();
    let malformed = tx
        .query_row(
            "SELECT COUNT(*) FROM memories \
             WHERE deleted = 0 AND scope = ?1 \
               AND (accessed_at IS NULL OR datetime(accessed_at) IS NULL)",
            [scope],
            |row| row.get::<_, i64>(0),
        )
        .map_err(LibreFangError::memory)?;
    if malformed > 0 {
        warn!(
            scope = label,
            malformed, "Memory decay skipped rows with invalid accessed_at"
        );
    }
    let deleted = tx
        .execute(
            "UPDATE memories \
             SET deleted = 1, deleted_at = ?3 \
             WHERE deleted = 0 AND scope = ?1 \
               AND datetime(accessed_at) < datetime(?2)",
            rusqlite::params![scope, cutoff_str, now_unix],
        )
        .map_err(LibreFangError::memory)?;
    if deleted > 0 {
        debug!(scope = label, deleted, cutoff = %cutoff_str, "Soft-deleted stale memories");
    }
    Ok(deleted)
}

/// Hard-delete memories that have been soft-deleted for longer than
/// `older_than_days`. Reclaims the embedding BLOB which would otherwise
/// stay in the row forever (#3467).
///
/// Rows with `deleted_at = NULL` (soft-deleted before v29 migration, or
/// never decayed) are ignored — operators can re-touch them with a manual
/// `UPDATE memories SET deleted_at = strftime('%s','now')` if desired.
///
/// Returns the number of rows hard-deleted.
pub fn prune_soft_deleted_memories(
    pool: &Pool<SqliteConnectionManager>,
    older_than_days: u64,
) -> LibreFangResult<usize> {
    if older_than_days == 0 {
        return Ok(0);
    }
    let db = pool.get().map_err(LibreFangError::memory)?;
    let cutoff = Utc::now().timestamp() - (older_than_days as i64) * 86_400;
    let pruned = db
        .execute(
            "DELETE FROM memories \
             WHERE deleted = 1 AND deleted_at IS NOT NULL AND deleted_at < ?1",
            rusqlite::params![cutoff],
        )
        .map_err(LibreFangError::memory)?;
    if pruned > 0 {
        info!(
            pruned,
            older_than_days, "Pruned soft-deleted memories (hard delete)"
        );
    }
    Ok(pruned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;
    use r2d2::Pool;
    use r2d2_sqlite::SqliteConnectionManager;
    use rusqlite::Connection;

    fn make_pool() -> Pool<SqliteConnectionManager> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        pool
    }

    /// Helper: insert a memory with a specific scope and accessed_at timestamp.
    fn insert_memory(conn: &Connection, id: &str, scope: &str, accessed_at: &str) {
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, 1.0, '{}', ?6, ?7, 0, 0)",
            rusqlite::params![
                id,
                "00000000-0000-0000-0000-000000000001",
                format!("test content for {id}"),
                "\"System\"",
                scope,
                accessed_at,
                accessed_at,
            ],
        )
        .unwrap();
    }

    /// Count non-deleted memories.
    fn count_memories(conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE deleted = 0",
            [],
            |row| row.get::<_, i64>(0).map(|v| v as usize),
        )
        .unwrap()
    }

    #[test]
    fn test_decay_deletes_old_session_memories() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        // Insert a session memory with old accessed_at (10 days ago)
        let old_time = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        insert_memory(&conn, "old-session", "session_memory", &old_time);

        // Insert a recent session memory (1 day ago)
        let recent_time = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        insert_memory(&conn, "new-session", "session_memory", &recent_time);

        assert_eq!(count_memories(&conn), 2);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&pool, &config).unwrap();
        assert_eq!(deleted, 1);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 1);

        // Verify the remaining memory is the recent one
        let remaining_id: String = db
            .query_row("SELECT id FROM memories WHERE deleted = 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining_id, "new-session");
    }

    /// #7911: `episodic` is the scope the agent loop writes on every turn and the table default, and before this change `run_decay` did not name it — so the highest-volume scope in a real store was the one scope with no exit at all.
    #[test]
    fn episodic_memories_expire_after_their_ttl() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        let old_time = (Utc::now() - chrono::Duration::days(120)).to_rfc3339();
        insert_memory(&conn, "old-episodic", "episodic", &old_time);
        let recent_time = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        insert_memory(&conn, "recent-episodic", "episodic", &recent_time);
        assert_eq!(count_memories(&conn), 2);
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 90,
            decay_interval_hours: 1,
        };
        assert_eq!(run_decay(&pool, &config).unwrap(), 1);

        let db = pool.get().unwrap();
        let remaining: String = db
            .query_row("SELECT id FROM memories WHERE deleted = 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, "recent-episodic");
    }

    /// The TTL is measured against `accessed_at`, which recall refreshes, so a row that keeps being retrieved keeps living however old it is.
    /// This is the property that makes a time-based policy safe for a scope whose whole job is "what did we talk about".
    #[test]
    fn episodic_ttl_is_measured_from_last_access_not_creation() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        // created_at is 400 days old, accessed_at is yesterday.
        let created = (Utc::now() - chrono::Duration::days(400)).to_rfc3339();
        let accessed = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted)
             VALUES ('hot-row', '00000000-0000-0000-0000-000000000001', 'still useful', '\"System\"', 'episodic', 1.0, '{}', ?1, ?2, 63, 0)",
            rusqlite::params![created, accessed],
        )
        .unwrap();
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 0,
            agent_ttl_days: 0,
            episodic_ttl_days: 90,
            decay_interval_hours: 1,
        };
        assert_eq!(run_decay(&pool, &config).unwrap(), 0);
        assert_eq!(count_memories(&pool.get().unwrap()), 1);
    }

    /// A zero TTL disables expiry for the episodic scope only — the other scopes still sweep, so the knob is per-scope and not a master switch.
    #[test]
    fn episodic_ttl_zero_disables_only_the_episodic_scope() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        let old_time = (Utc::now() - chrono::Duration::days(365)).to_rfc3339();
        insert_memory(&conn, "old-episodic", "episodic", &old_time);
        insert_memory(&conn, "old-agent", "agent_memory", &old_time);
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };
        assert_eq!(run_decay(&pool, &config).unwrap(), 1);

        let db = pool.get().unwrap();
        let remaining: String = db
            .query_row("SELECT id FROM memories WHERE deleted = 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, "old-episodic");
    }

    /// Decay is a soft delete, so the existing `prune_soft_deleted_memories` retention sweep is what finally reclaims the row and its embedding BLOB.
    /// Without this the episodic exit would only be half-built: rows would stop being recalled but the bytes would stay forever.
    #[test]
    fn expired_episodic_rows_are_hard_deleted_by_the_retention_sweep() {
        let pool = make_pool();
        let conn = pool.get().unwrap();
        let old_time = (Utc::now() - chrono::Duration::days(365)).to_rfc3339();
        insert_memory(&conn, "old-episodic", "episodic", &old_time);
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 0,
            agent_ttl_days: 0,
            episodic_ttl_days: 90,
            decay_interval_hours: 1,
        };
        assert_eq!(run_decay(&pool, &config).unwrap(), 1);

        // Backdate deleted_at so the retention sweep considers it eligible.
        {
            let db = pool.get().unwrap();
            db.execute(
                "UPDATE memories SET deleted_at = ?1 WHERE id = 'old-episodic'",
                rusqlite::params![Utc::now().timestamp() - 31 * 86_400],
            )
            .unwrap();
        }
        assert_eq!(prune_soft_deleted_memories(&pool, 30).unwrap(), 1);

        let total: i64 = pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 0, "the row and its BLOB are gone, not just hidden");
    }

    #[test]
    fn test_decay_preserves_user_memories() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        // Insert a USER memory with very old accessed_at (100 days ago)
        let old_time = (Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        insert_memory(&conn, "old-user", "user_memory", &old_time);

        assert_eq!(count_memories(&conn), 1);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&pool, &config).unwrap();
        assert_eq!(deleted, 0);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_decay_deletes_old_agent_memories() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        // Insert an AGENT memory accessed 40 days ago (> 30 day TTL)
        let old_time = (Utc::now() - chrono::Duration::days(40)).to_rfc3339();
        insert_memory(&conn, "old-agent", "agent_memory", &old_time);

        // Insert an AGENT memory accessed 10 days ago (< 30 day TTL)
        let recent_time = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        insert_memory(&conn, "new-agent", "agent_memory", &recent_time);

        assert_eq!(count_memories(&conn), 2);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&pool, &config).unwrap();
        assert_eq!(deleted, 1);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_decay_disabled_does_nothing() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        let old_time = (Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        insert_memory(&conn, "old-session", "session_memory", &old_time);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: false,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&pool, &config).unwrap();
        assert_eq!(deleted, 0);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_access_resets_decay_timer() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        // Insert a session memory with old accessed_at (10 days ago)
        let old_time = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        insert_memory(&conn, "accessed-session", "session_memory", &old_time);

        // Simulate an access by updating accessed_at to now
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE memories SET accessed_at = ?1 WHERE id = ?2",
            rusqlite::params![now, "accessed-session"],
        )
        .unwrap();

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };

        // Should NOT be decayed because accessed_at was refreshed
        let deleted = run_decay(&pool, &config).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_decay_mixed_scopes() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        let old_time = (Utc::now() - chrono::Duration::days(50)).to_rfc3339();

        // All very old, but different scopes
        insert_memory(&conn, "user-old", "user_memory", &old_time);
        insert_memory(&conn, "session-old", "session_memory", &old_time);
        insert_memory(&conn, "agent-old", "agent_memory", &old_time);

        assert_eq!(count_memories(&conn), 3);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&pool, &config).unwrap();
        // session_memory and agent_memory should be deleted, user_memory preserved
        assert_eq!(deleted, 2);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 1);

        let remaining_id: String = db
            .query_row("SELECT id FROM memories WHERE deleted = 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining_id, "user-old");
    }

    #[test]
    fn test_decay_rolls_back_all_scopes_when_one_update_fails() {
        let pool = make_pool();
        let conn = pool.get().unwrap();
        let old_time = (Utc::now() - chrono::Duration::days(50)).to_rfc3339();
        insert_memory(&conn, "session-old", "session_memory", &old_time);
        insert_memory(&conn, "agent-old", "agent_memory", &old_time);
        conn.execute_batch(
            "CREATE TRIGGER reject_agent_decay
             BEFORE UPDATE OF deleted ON memories
             WHEN OLD.scope = 'agent_memory'
             BEGIN
                 SELECT RAISE(ABORT, 'reject agent decay');
             END;",
        )
        .unwrap();
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };
        assert!(run_decay(&pool, &config).is_err());

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 2, "SESSION decay must roll back");
    }

    #[test]
    fn test_decay_leaves_malformed_timestamps_for_operator_repair() {
        let pool = make_pool();
        let conn = pool.get().unwrap();
        insert_memory(&conn, "malformed", "session_memory", "not-a-timestamp");
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };
        assert_eq!(run_decay(&pool, &config).unwrap(), 0);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_zero_ttl_disables_expiry_for_each_scope() {
        let pool = make_pool();
        let conn = pool.get().unwrap();
        let old_time = (Utc::now() - chrono::Duration::days(50)).to_rfc3339();
        insert_memory(&conn, "session-old", "session_memory", &old_time);
        insert_memory(&conn, "agent-old", "agent_memory", &old_time);
        drop(conn);

        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 0,
            agent_ttl_days: 0,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };
        assert_eq!(run_decay(&pool, &config).unwrap(), 0);

        let db = pool.get().unwrap();
        assert_eq!(count_memories(&db), 2);
    }

    /// Total row count regardless of `deleted` flag.
    fn count_total(conn: &Connection) -> usize {
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| {
            row.get::<_, i64>(0).map(|v| v as usize)
        })
        .unwrap()
    }

    #[test]
    fn test_decay_soft_deletes_does_not_hard_delete() {
        let pool = make_pool();
        let conn = pool.get().unwrap();
        let old_time = (Utc::now() - chrono::Duration::days(40)).to_rfc3339();
        insert_memory(&conn, "stale", "agent_memory", &old_time);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            episodic_ttl_days: 0,
            decay_interval_hours: 1,
        };
        run_decay(&pool, &config).unwrap();

        let db = pool.get().unwrap();
        // Row is still present, just flagged.
        assert_eq!(count_total(&db), 1);
        assert_eq!(count_memories(&db), 0);
        let deleted_at: Option<i64> = db
            .query_row(
                "SELECT deleted_at FROM memories WHERE id = 'stale'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            deleted_at.is_some(),
            "decay must stamp deleted_at for retention sweep"
        );
    }

    #[test]
    fn test_prune_soft_deleted_memories_hard_deletes_old() {
        let pool = make_pool();
        let conn = pool.get().unwrap();

        let now_unix = Utc::now().timestamp();
        let old_unix = now_unix - 60 * 86_400; // 60 days ago
        let recent_unix = now_unix - 86_400; // 1 day ago

        // One old soft-deleted row, one recent soft-deleted row, one alive row.
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, deleted_at)
             VALUES ('old-soft', 'a', 'x', '\"System\"', 'agent_memory', 1.0, '{}', ?1, ?1, 0, 1, ?2)",
            rusqlite::params![Utc::now().to_rfc3339(), old_unix],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, deleted_at)
             VALUES ('recent-soft', 'a', 'x', '\"System\"', 'agent_memory', 1.0, '{}', ?1, ?1, 0, 1, ?2)",
            rusqlite::params![Utc::now().to_rfc3339(), recent_unix],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted)
             VALUES ('alive', 'a', 'x', '\"System\"', 'agent_memory', 1.0, '{}', ?1, ?1, 0, 0)",
            rusqlite::params![Utc::now().to_rfc3339()],
        )
        .unwrap();

        assert_eq!(count_total(&conn), 3);

        // conn returned to pool here; use pool for function calls
        drop(conn);
        let pruned = prune_soft_deleted_memories(&pool, 30).unwrap();
        assert_eq!(pruned, 1, "only the 60-day-old soft-deleted row should go");

        let db = pool.get().unwrap();
        assert_eq!(count_total(&db), 2);
        // The alive row and the recent-soft row remain.
        let ids: Vec<String> = db
            .prepare("SELECT id FROM memories ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(ids, vec!["alive".to_string(), "recent-soft".to_string()]);
    }

    #[test]
    fn test_prune_soft_deleted_memories_zero_disabled() {
        let pool = make_pool();
        let conn = pool.get().unwrap();
        // conn returned to pool here; use pool for function calls
        drop(conn);
        // Even if there's nothing to prune, 0 must short-circuit and not error.
        let pruned = prune_soft_deleted_memories(&pool, 0).unwrap();
        assert_eq!(pruned, 0);
    }

    /// `SemanticStore::forget*` must stamp `deleted_at`; without it, the
    /// retention sweep (filter `deleted_at IS NOT NULL`) would skip user- /
    /// API-initiated deletions forever, leaking the embedding BLOB.
    #[test]
    fn forget_variants_stamp_deleted_at_so_prune_sees_them() {
        use crate::semantic::SemanticStore;
        use librefang_types::agent::AgentId;
        use librefang_types::memory::MemorySource;
        use std::collections::HashMap;

        let pool = make_pool();
        let store = SemanticStore::new(pool.clone());
        let agent = AgentId::new();

        let mid_forget = store
            .remember(
                agent,
                "single forget",
                MemorySource::Conversation,
                "agent_memory",
                HashMap::new(),
            )
            .unwrap();
        let mid_by_agent = store
            .remember(
                agent,
                "agent-wide forget",
                MemorySource::Conversation,
                "agent_memory",
                HashMap::new(),
            )
            .unwrap();
        let mid_by_scope = store
            .remember(
                agent,
                "scope forget",
                MemorySource::Conversation,
                "session_memory",
                HashMap::new(),
            )
            .unwrap();

        store.forget(mid_forget).unwrap();
        store.forget_by_agent(agent).unwrap();
        store.forget_by_scope(agent, "session_memory").unwrap();

        let db = pool.get().unwrap();
        for id in [mid_forget, mid_by_agent, mid_by_scope] {
            let (deleted, deleted_at): (i64, Option<i64>) = db
                .query_row(
                    "SELECT deleted, deleted_at FROM memories WHERE id = ?1",
                    rusqlite::params![id.0.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(deleted, 1, "forget* must soft-delete {id:?}");
            assert!(
                deleted_at.is_some(),
                "forget* must stamp deleted_at for {id:?} so prune sweep picks it up"
            );
        }

        // Wind deleted_at backwards so a 7-day prune captures the rows.
        let ancient = Utc::now().timestamp() - 30 * 86_400;
        db.execute(
            "UPDATE memories SET deleted_at = ?1 WHERE deleted = 1",
            rusqlite::params![ancient],
        )
        .unwrap();
        drop(db);

        let pruned = prune_soft_deleted_memories(&pool, 7).unwrap();
        assert_eq!(
            pruned, 3,
            "all three forget* variants must produce prune-eligible rows"
        );
    }
}
