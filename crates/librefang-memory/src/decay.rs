//! Time-based memory decay — soft-deletes stale memories based on scope TTL.
//!
//! Scope rules:
//! - **USER**: Never decays (permanent user knowledge).
//! - **SESSION**: Decays after `session_ttl_days` of no access.
//! - **AGENT**: Decays after `agent_ttl_days` of no access.
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
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

/// Run time-based decay on the memories table.
///
/// Soft-deletes SESSION and AGENT scope memories whose `accessed_at` is older
/// than the configured TTL. USER scope memories are never touched.
///
/// `accessed_at` is stored as RFC3339; rather than rely on lexicographic
/// string comparison (which is wrong as soon as offsets / `Z` vs `+00:00` /
/// fractional-second precision diverge), we wrap both sides in
/// `datetime(...)` so SQLite parses them as real timestamps before comparing.
///
/// Returns the number of memories soft-deleted.
pub fn run_decay(
    conn: &Arc<Mutex<Connection>>,
    config: &MemoryDecayConfig,
) -> LibreFangResult<usize> {
    if !config.enabled {
        return Ok(0);
    }

    let db = conn
        .lock()
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;

    let now = Utc::now();
    let now_unix = now.timestamp();
    let mut total_deleted: usize = 0;

    // Decay SESSION scope memories — soft-delete only.
    if config.session_ttl_days > 0 {
        let cutoff = now - chrono::Duration::days(i64::from(config.session_ttl_days));
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = db
            .execute(
                "UPDATE memories \
                 SET deleted = 1, deleted_at = ?3 \
                 WHERE deleted = 0 AND scope = ?1 \
                   AND datetime(accessed_at) < datetime(?2)",
                rusqlite::params!["session_memory", cutoff_str, now_unix],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        if deleted > 0 {
            debug!(scope = "SESSION", deleted, cutoff = %cutoff_str, "Soft-deleted stale memories");
        }
        total_deleted += deleted;
    }

    // Decay AGENT scope memories — soft-delete only.
    if config.agent_ttl_days > 0 {
        let cutoff = now - chrono::Duration::days(i64::from(config.agent_ttl_days));
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = db
            .execute(
                "UPDATE memories \
                 SET deleted = 1, deleted_at = ?3 \
                 WHERE deleted = 0 AND scope = ?1 \
                   AND datetime(accessed_at) < datetime(?2)",
                rusqlite::params!["agent_memory", cutoff_str, now_unix],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        if deleted > 0 {
            debug!(scope = "AGENT", deleted, cutoff = %cutoff_str, "Soft-deleted stale memories");
        }
        total_deleted += deleted;
    }

    if total_deleted > 0 {
        info!(total_deleted, "Memory decay sweep completed");
    }

    Ok(total_deleted)
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
    conn: &Arc<Mutex<Connection>>,
    older_than_days: u64,
) -> LibreFangResult<usize> {
    if older_than_days == 0 {
        return Ok(0);
    }
    let db = conn
        .lock()
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
    let cutoff = Utc::now().timestamp() - (older_than_days as i64) * 86_400;
    let pruned = db
        .execute(
            "DELETE FROM memories \
             WHERE deleted = 1 AND deleted_at IS NOT NULL AND deleted_at < ?1",
            rusqlite::params![cutoff],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
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
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Insert a session memory with old accessed_at (10 days ago)
        let old_time = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        insert_memory(&conn, "old-session", "session_memory", &old_time);

        // Insert a recent session memory (1 day ago)
        let recent_time = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        insert_memory(&conn, "new-session", "session_memory", &recent_time);

        assert_eq!(count_memories(&conn), 2);

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&shared, &config).unwrap();
        assert_eq!(deleted, 1);

        let db = shared.lock().unwrap();
        assert_eq!(count_memories(&db), 1);

        // Verify the remaining memory is the recent one
        let remaining_id: String = db
            .query_row("SELECT id FROM memories WHERE deleted = 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining_id, "new-session");
    }

    #[test]
    fn test_decay_preserves_user_memories() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Insert a USER memory with very old accessed_at (100 days ago)
        let old_time = (Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        insert_memory(&conn, "old-user", "user_memory", &old_time);

        assert_eq!(count_memories(&conn), 1);

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&shared, &config).unwrap();
        assert_eq!(deleted, 0);

        let db = shared.lock().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_decay_deletes_old_agent_memories() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Insert an AGENT memory accessed 40 days ago (> 30 day TTL)
        let old_time = (Utc::now() - chrono::Duration::days(40)).to_rfc3339();
        insert_memory(&conn, "old-agent", "agent_memory", &old_time);

        // Insert an AGENT memory accessed 10 days ago (< 30 day TTL)
        let recent_time = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        insert_memory(&conn, "new-agent", "agent_memory", &recent_time);

        assert_eq!(count_memories(&conn), 2);

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&shared, &config).unwrap();
        assert_eq!(deleted, 1);

        let db = shared.lock().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_decay_disabled_does_nothing() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let old_time = (Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        insert_memory(&conn, "old-session", "session_memory", &old_time);

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: false,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&shared, &config).unwrap();
        assert_eq!(deleted, 0);

        let db = shared.lock().unwrap();
        assert_eq!(count_memories(&db), 1);
    }

    #[test]
    fn test_access_resets_decay_timer() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

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

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };

        // Should NOT be decayed because accessed_at was refreshed
        let deleted = run_decay(&shared, &config).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_decay_mixed_scopes() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let old_time = (Utc::now() - chrono::Duration::days(50)).to_rfc3339();

        // All very old, but different scopes
        insert_memory(&conn, "user-old", "user_memory", &old_time);
        insert_memory(&conn, "session-old", "session_memory", &old_time);
        insert_memory(&conn, "agent-old", "agent_memory", &old_time);

        assert_eq!(count_memories(&conn), 3);

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };

        let deleted = run_decay(&shared, &config).unwrap();
        // session_memory and agent_memory should be deleted, user_memory preserved
        assert_eq!(deleted, 2);

        let db = shared.lock().unwrap();
        assert_eq!(count_memories(&db), 1);

        let remaining_id: String = db
            .query_row("SELECT id FROM memories WHERE deleted = 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining_id, "user-old");
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
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let old_time = (Utc::now() - chrono::Duration::days(40)).to_rfc3339();
        insert_memory(&conn, "stale", "agent_memory", &old_time);

        let shared = Arc::new(Mutex::new(conn));
        let config = MemoryDecayConfig {
            enabled: true,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        };
        run_decay(&shared, &config).unwrap();

        let db = shared.lock().unwrap();
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
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let now_unix = Utc::now().timestamp();
        let old_unix = now_unix - 60 * 86_400; // 60 days ago
        let recent_unix = now_unix - 1 * 86_400; // 1 day ago

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

        let shared = Arc::new(Mutex::new(conn));
        let pruned = prune_soft_deleted_memories(&shared, 30).unwrap();
        assert_eq!(pruned, 1, "only the 60-day-old soft-deleted row should go");

        let db = shared.lock().unwrap();
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
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let shared = Arc::new(Mutex::new(conn));
        // Even if there's nothing to prune, 0 must short-circuit and not error.
        let pruned = prune_soft_deleted_memories(&shared, 0).unwrap();
        assert_eq!(pruned, 0);
    }
}
