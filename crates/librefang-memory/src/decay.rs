//! Time-based memory decay — deletes stale memories based on scope TTL.
//!
//! Scope rules:
//! - **USER**: Never decays (permanent user knowledge).
//! - **SESSION**: Decays after `session_ttl_days` of no access.
//! - **AGENT**: Decays after `agent_ttl_days` of no access.
//!
//! Accessing a memory (via search/recall) resets the decay timer by updating
//! `accessed_at`, which is already handled by `SemanticStore::recall_with_embedding`.

use chrono::Utc;
use librefang_types::config::MemoryDecayConfig;
use librefang_types::error::{LibreFangError, LibreFangResult};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

/// Run time-based decay on the memories table.
///
/// Deletes (hard-delete) SESSION and AGENT scope memories whose `accessed_at`
/// timestamp is older than the configured TTL. USER scope memories are never
/// touched.
///
/// Returns the number of memories deleted.
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
    let mut total_deleted: usize = 0;

    // Decay SESSION scope memories
    if config.session_ttl_days > 0 {
        let cutoff = now - chrono::Duration::days(i64::from(config.session_ttl_days));
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = db
            .execute(
                "DELETE FROM memories WHERE deleted = 0 AND scope = ?1 AND accessed_at < ?2",
                rusqlite::params!["session_memory", cutoff_str],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        if deleted > 0 {
            debug!(scope = "SESSION", deleted, cutoff = %cutoff_str, "Decayed stale memories");
        }
        total_deleted += deleted;
    }

    // Decay AGENT scope memories
    if config.agent_ttl_days > 0 {
        let cutoff = now - chrono::Duration::days(i64::from(config.agent_ttl_days));
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = db
            .execute(
                "DELETE FROM memories WHERE deleted = 0 AND scope = ?1 AND accessed_at < ?2",
                rusqlite::params!["agent_memory", cutoff_str],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        if deleted > 0 {
            debug!(scope = "AGENT", deleted, cutoff = %cutoff_str, "Decayed stale memories");
        }
        total_deleted += deleted;
    }

    if total_deleted > 0 {
        info!(total_deleted, "Memory decay sweep completed");
    }

    Ok(total_deleted)
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
}
