//! SQLite schema creation and migration.
//!
//! Creates all tables needed by the memory substrate on first boot.

use rusqlite::Connection;

/// Current schema version.
const SCHEMA_VERSION: u32 = 23;

/// Run all migrations to bring the database up to date.
pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    let current_version = get_schema_version(conn);

    if current_version < 1 {
        migrate_v1(conn)?;
    }

    if current_version < 2 {
        migrate_v2(conn)?;
    }

    if current_version < 3 {
        migrate_v3(conn)?;
    }

    if current_version < 4 {
        migrate_v4(conn)?;
    }

    if current_version < 5 {
        migrate_v5(conn)?;
    }

    if current_version < 6 {
        migrate_v6(conn)?;
    }

    if current_version < 7 {
        migrate_v7(conn)?;
    }

    if current_version < 8 {
        migrate_v8(conn)?;
    }

    if current_version < 9 {
        migrate_v9(conn)?;
    }

    if current_version < 10 {
        migrate_v10(conn)?;
    }

    if current_version < 11 {
        migrate_v11(conn)?;
    }

    if current_version < 12 {
        migrate_v12(conn)?;
    }

    if current_version < 13 {
        migrate_v13(conn)?;
    }

    if current_version < 14 {
        migrate_v14(conn)?;
    }

    if current_version < 15 {
        migrate_v15(conn)?;
    }

    if current_version < 16 {
        migrate_v16(conn)?;
    }

    if current_version < 17 {
        migrate_v17(conn)?;
    }

    if current_version < 18 {
        migrate_v18(conn)?;
    }

    if current_version < 19 {
        migrate_v19(conn)?;
    }

    if current_version < 20 {
        migrate_v20(conn)?;
    }

    if current_version < 21 {
        migrate_v21(conn)?;
    }

    if current_version < 22 {
        migrate_v22(conn)?;
    }

    if current_version < 23 {
        migrate_v23(conn)?;
    }

    set_schema_version(conn, SCHEMA_VERSION)?;
    Ok(())
}

/// Get the current schema version from the database.
fn get_schema_version(conn: &Connection) -> u32 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0)
}

/// Check if a column exists in a table (SQLite has no ADD COLUMN IF NOT EXISTS).
fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let names: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    names.iter().any(|n| n == column)
}

/// Set the schema version in the database.
fn set_schema_version(conn: &Connection, version: u32) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "user_version", version)
}

/// Version 1: Create all core tables.
fn migrate_v1(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        -- Agent registry
        CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            manifest BLOB NOT NULL,
            state TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Session history
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            messages BLOB NOT NULL,
            context_window_tokens INTEGER DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Event log
        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            source_agent TEXT NOT NULL,
            target TEXT NOT NULL,
            payload BLOB NOT NULL,
            timestamp TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_events_source ON events(source_agent);

        -- Key-value store (per-agent)
        CREATE TABLE IF NOT EXISTS kv_store (
            agent_id TEXT NOT NULL,
            key TEXT NOT NULL,
            value BLOB NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (agent_id, key)
        );

        -- Task queue
        CREATE TABLE IF NOT EXISTS task_queue (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            task_type TEXT NOT NULL,
            payload BLOB NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            priority INTEGER NOT NULL DEFAULT 0,
            scheduled_at TEXT,
            created_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_task_status_priority ON task_queue(status, priority DESC);

        -- Semantic memories
        CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            content TEXT NOT NULL,
            source TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'episodic',
            confidence REAL NOT NULL DEFAULT 1.0,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            accessed_at TEXT NOT NULL,
            access_count INTEGER NOT NULL DEFAULT 0,
            deleted INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_memories_agent ON memories(agent_id);
        CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);

        -- Knowledge graph entities
        CREATE TABLE IF NOT EXISTS entities (
            id TEXT PRIMARY KEY,
            entity_type TEXT NOT NULL,
            name TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Knowledge graph relations
        CREATE TABLE IF NOT EXISTS relations (
            id TEXT PRIMARY KEY,
            source_entity TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            target_entity TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            confidence REAL NOT NULL DEFAULT 1.0,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_relations_source ON relations(source_entity);
        CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_entity);
        CREATE INDEX IF NOT EXISTS idx_relations_type ON relations(relation_type);

        -- Migration tracking
        CREATE TABLE IF NOT EXISTS migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL,
            description TEXT
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (1, datetime('now'), 'Initial schema');
        ",
    )?;
    Ok(())
}

/// Version 2: Add collaboration columns to task_queue for agent task delegation.
fn migrate_v2(conn: &Connection) -> Result<(), rusqlite::Error> {
    // SQLite requires one ALTER TABLE per statement; check before adding
    let cols = [
        ("title", "TEXT DEFAULT ''"),
        ("description", "TEXT DEFAULT ''"),
        ("assigned_to", "TEXT DEFAULT ''"),
        ("created_by", "TEXT DEFAULT ''"),
        ("result", "TEXT DEFAULT ''"),
    ];
    for (name, typedef) in &cols {
        if !column_exists(conn, "task_queue", name) {
            conn.execute(
                &format!("ALTER TABLE task_queue ADD COLUMN {} {}", name, typedef),
                [],
            )?;
        }
    }

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (2, datetime('now'), 'Add collaboration columns to task_queue')",
        [],
    )?;

    Ok(())
}

/// Version 3: Add embedding column to memories table for vector search.
fn migrate_v3(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "memories", "embedding") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN embedding BLOB DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (3, datetime('now'), 'Add embedding column to memories')",
        [],
    )?;
    Ok(())
}

/// Version 4: Add usage_events table for cost tracking and metering.
fn migrate_v4(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS usage_events (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0,
            tool_calls INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_usage_agent_time ON usage_events(agent_id, timestamp);
        CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON usage_events(timestamp);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (4, datetime('now'), 'Add usage_events table for cost tracking');
        ",
    )?;
    Ok(())
}

/// Version 5: Add canonical_sessions table for cross-channel persistent memory.
fn migrate_v5(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS canonical_sessions (
            agent_id TEXT PRIMARY KEY,
            messages BLOB NOT NULL,
            compaction_cursor INTEGER NOT NULL DEFAULT 0,
            compacted_summary TEXT,
            updated_at TEXT NOT NULL
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (5, datetime('now'), 'Add canonical_sessions for cross-channel memory');
        ",
    )?;
    Ok(())
}

/// Version 6: Add label column to sessions table.
fn migrate_v6(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Check if column already exists before ALTER (SQLite has no ADD COLUMN IF NOT EXISTS)
    if !column_exists(conn, "sessions", "label") {
        conn.execute("ALTER TABLE sessions ADD COLUMN label TEXT", [])?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (6, datetime('now'), 'Add label column to sessions for human-readable labels')",
        [],
    )?;
    Ok(())
}

/// Version 7: Add paired_devices table for device pairing persistence.
fn migrate_v7(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS paired_devices (
            device_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            platform TEXT NOT NULL,
            paired_at TEXT NOT NULL,
            last_seen TEXT NOT NULL,
            push_token TEXT
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (7, datetime('now'), 'Add paired_devices table for device pairing');
        ",
    )?;
    Ok(())
}

/// Version 8: Add audit_entries table for persistent Merkle audit trail.
fn migrate_v8(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS audit_entries (
            seq INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            action TEXT NOT NULL,
            detail TEXT NOT NULL,
            outcome TEXT NOT NULL,
            prev_hash TEXT NOT NULL,
            hash TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_agent ON audit_entries(agent_id);
        CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_entries(timestamp);
        CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_entries(action);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (8, datetime('now'), 'Add audit_entries table for persistent Merkle audit trail');
        ",
    )?;
    Ok(())
}

/// Version 9: Add performance indexes for proactive memory queries.
fn migrate_v9(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        -- Composite index for recall ordering (confidence DESC, accessed_at DESC)
        CREATE INDEX IF NOT EXISTS idx_memories_confidence_accessed
            ON memories(deleted, agent_id, confidence DESC, accessed_at DESC);

        -- Index for confidence decay queries (accessed_at filtering on non-deleted)
        CREATE INDEX IF NOT EXISTS idx_memories_decay
            ON memories(deleted, accessed_at);

        -- Index for lowest-confidence eviction queries
        CREATE INDEX IF NOT EXISTS idx_memories_eviction
            ON memories(deleted, agent_id, confidence ASC, created_at ASC);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (9, datetime('now'), 'Add performance indexes for proactive memory queries');
        ",
    )?;
    Ok(())
}

/// Version 10: Add agent_id to entities and relations for per-agent cleanup.
fn migrate_v10(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        ALTER TABLE entities ADD COLUMN agent_id TEXT NOT NULL DEFAULT '';
        ALTER TABLE relations ADD COLUMN agent_id TEXT NOT NULL DEFAULT '';
        CREATE INDEX IF NOT EXISTS idx_entities_agent ON entities(agent_id);
        CREATE INDEX IF NOT EXISTS idx_relations_agent ON relations(agent_id);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (10, datetime('now'), 'Add agent_id to entities and relations');
        ",
    )?;
    Ok(())
}

/// Version 11: Add index on entities.name for name-based JOIN lookups.
fn migrate_v11(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (11, datetime('now'), 'Add index on entities.name for knowledge graph queries');
        ",
    )?;
    Ok(())
}

/// Version 12: Add FTS5 virtual table for full-text session search.
fn migrate_v12(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
            session_id UNINDEXED,
            agent_id UNINDEXED,
            content
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (12, datetime('now'), 'Add FTS5 virtual table for full-text session search');
        ",
    )?;
    Ok(())
}

/// Version 13: Add prompt versioning and A/B testing tables.
fn migrate_v13(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        -- Prompt versions: stores version history for agent prompts
        CREATE TABLE IF NOT EXISTS prompt_versions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            version INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            system_prompt TEXT NOT NULL,
            tools TEXT NOT NULL,
            variables TEXT NOT NULL,
            created_at TEXT NOT NULL,
            created_by TEXT NOT NULL,
            is_active INTEGER NOT NULL DEFAULT 0,
            description TEXT,
            UNIQUE(agent_id, version)
        );

        -- Prompt experiments: A/B experiment definitions
        CREATE TABLE IF NOT EXISTS prompt_experiments (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            status TEXT NOT NULL,
            traffic_split TEXT NOT NULL,
            success_criteria TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY(agent_id) REFERENCES agents(id)
        );

        -- Experiment variants: variants within experiments
        CREATE TABLE IF NOT EXISTS experiment_variants (
            id TEXT PRIMARY KEY,
            experiment_id TEXT NOT NULL,
            name TEXT NOT NULL,
            prompt_version_id TEXT NOT NULL,
            description TEXT,
            FOREIGN KEY(experiment_id) REFERENCES prompt_experiments(id),
            FOREIGN KEY(prompt_version_id) REFERENCES prompt_versions(id)
        );

        -- Experiment metrics: aggregated metrics per variant
        CREATE TABLE IF NOT EXISTS experiment_metrics (
            id TEXT PRIMARY KEY,
            experiment_id TEXT NOT NULL,
            variant_id TEXT NOT NULL,
            total_requests INTEGER NOT NULL DEFAULT 0,
            successful_requests INTEGER NOT NULL DEFAULT 0,
            failed_requests INTEGER NOT NULL DEFAULT 0,
            total_latency_ms INTEGER NOT NULL DEFAULT 0,
            total_cost_usd REAL NOT NULL DEFAULT 0,
            last_updated TEXT NOT NULL,
            FOREIGN KEY(experiment_id) REFERENCES prompt_experiments(id),
            FOREIGN KEY(variant_id) REFERENCES experiment_variants(id)
        );

        -- Indexes for prompt versioning tables
        CREATE INDEX IF NOT EXISTS idx_prompt_versions_agent ON prompt_versions(agent_id);
        CREATE INDEX IF NOT EXISTS idx_prompt_versions_active ON prompt_versions(agent_id, is_active);
        CREATE INDEX IF NOT EXISTS idx_experiments_agent ON prompt_experiments(agent_id);
        CREATE INDEX IF NOT EXISTS idx_experiments_status ON prompt_experiments(status);
        CREATE INDEX IF NOT EXISTS idx_experiment_variants_experiment ON experiment_variants(experiment_id);
        CREATE INDEX IF NOT EXISTS idx_experiment_metrics_variant ON experiment_metrics(variant_id);
        ",
    )
}

/// Version 14: Add latency_ms column to usage_events for model performance tracking.
fn migrate_v14(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "usage_events", "latency_ms") {
        conn.execute(
            "ALTER TABLE usage_events ADD COLUMN latency_ms INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (14, datetime('now'), 'Add latency_ms column to usage_events')",
        [],
    )?;
    Ok(())
}

/// Version 15: Add multimodal memory columns for image URL, image embedding, and modality.
fn migrate_v15(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "memories", "image_url") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN image_url TEXT DEFAULT NULL",
            [],
        )?;
    }
    if !column_exists(conn, "memories", "image_embedding") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN image_embedding BLOB DEFAULT NULL",
            [],
        )?;
    }
    if !column_exists(conn, "memories", "modality") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN modality TEXT DEFAULT 'text'",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (15, datetime('now'), 'Add multimodal memory columns (image_url, image_embedding, modality)')",
        [],
    )?;
    Ok(())
}

/// v16: Add peer_id column to memories and sessions for per-user isolation.
fn migrate_v16(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "memories", "peer_id") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN peer_id TEXT DEFAULT NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memories_peer ON memories(agent_id, peer_id)",
            [],
        )?;
    }
    if !column_exists(conn, "sessions", "peer_id") {
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN peer_id TEXT DEFAULT NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_peer ON sessions(agent_id, peer_id)",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (16, datetime('now'), 'Add peer_id to memories and sessions for per-user isolation')",
        [],
    )?;
    Ok(())
}

/// V17: Persistent approval audit log.
fn migrate_v17(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS approval_audit (
            id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            action_summary TEXT NOT NULL DEFAULT '',
            risk_level TEXT NOT NULL DEFAULT 'low',
            decision TEXT NOT NULL,
            decided_by TEXT,
            decided_at TEXT NOT NULL,
            requested_at TEXT NOT NULL,
            feedback TEXT,
            second_factor_used INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_approval_audit_agent ON approval_audit(agent_id);
        CREATE INDEX IF NOT EXISTS idx_approval_audit_decided ON approval_audit(decided_at);
        ",
    )?;
    // Migration: add second_factor_used column (ignore error if already exists)
    let _ = conn.execute(
        "ALTER TABLE approval_audit ADD COLUMN second_factor_used INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(())
}

fn migrate_v18(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS totp_lockout (
            sender_id  TEXT    PRIMARY KEY,
            failures   INTEGER NOT NULL DEFAULT 0,
            locked_at  INTEGER             -- Unix timestamp (seconds) when lockout started, NULL if below threshold
        );",
    )
}

/// Version 19: Add `provider` column to usage_events so the metering engine
/// can enforce per-provider budget caps (issue #2316).
fn migrate_v19(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "usage_events", "provider") {
        conn.execute(
            "ALTER TABLE usage_events ADD COLUMN provider TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_usage_provider_time ON usage_events(provider, timestamp)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (19, datetime('now'), 'Add provider column for per-provider budgets')",
        [],
    )?;
    Ok(())
}

/// Version 20: Add `claimed_at` column to `task_queue` so the kernel can
/// detect and auto-reset stuck `in_progress` tasks whose worker LLM stalled
/// or crashed without calling `task_complete` (issue #2923 / #2926).
fn migrate_v20(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "task_queue", "claimed_at") {
        conn.execute(
            "ALTER TABLE task_queue ADD COLUMN claimed_at TEXT DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_task_status_claimed_at ON task_queue(status, claimed_at)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (20, datetime('now'), 'Add claimed_at column to task_queue for stuck-task auto-reset')",
        [],
    )?;
    Ok(())
}

/// Version 21: Add `retry_count` column to `task_queue` so the kernel sweep
/// can enforce `max_retries` and mark exhausted tasks as `failed`.
fn migrate_v21(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "task_queue", "retry_count") {
        conn.execute(
            "ALTER TABLE task_queue ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (21, datetime('now'), 'Add retry_count column to task_queue for max_retries enforcement')",
        [],
    )?;
    Ok(())
}

/// Version 22: Add user_id and channel columns to audit_entries for RBAC M1.
///
/// Both columns are nullable so pre-M1 entries (no user attribution) keep
/// verifying with their original Merkle hashes — the hash function omits
/// absent fields, so NULL columns produce the pre-migration hash unchanged.
fn migrate_v22(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "audit_entries", "user_id") {
        conn.execute("ALTER TABLE audit_entries ADD COLUMN user_id TEXT", [])?;
    }
    if !column_exists(conn, "audit_entries", "channel") {
        conn.execute("ALTER TABLE audit_entries ADD COLUMN channel TEXT", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_audit_user ON audit_entries(user_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_audit_channel ON audit_entries(channel)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (22, datetime('now'), 'Add user_id and channel columns to audit_entries for RBAC M1 attribution')",
        [],
    )?;
    Ok(())
}

/// Version 23 (RBAC M5): attribute usage events to a user / channel.
///
/// Adds two NULL-able columns to `usage_events` and indexes them so
/// `/api/budget/users` and `/api/budget/users/{id}` can roll spend up by
/// user without scanning the whole table. Pre-M5 rows return NULL — they
/// fall outside any per-user filter, which is the right default (cost
/// existed before the user attribution layer was added).
fn migrate_v23(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "usage_events", "user_id") {
        conn.execute("ALTER TABLE usage_events ADD COLUMN user_id TEXT", [])?;
    }
    if !column_exists(conn, "usage_events", "channel") {
        conn.execute("ALTER TABLE usage_events ADD COLUMN channel TEXT", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_usage_user_time ON usage_events(user_id, timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_usage_channel_time ON usage_events(channel, timestamp)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (23, datetime('now'), 'Add user_id and channel columns to usage_events for RBAC M5 per-user spend rollup')",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"agents".to_string()));
        assert!(tables.contains(&"sessions".to_string()));
        assert!(tables.contains(&"kv_store".to_string()));
        assert!(tables.contains(&"memories".to_string()));
        assert!(tables.contains(&"entities".to_string()));
        assert!(tables.contains(&"relations".to_string()));
    }

    #[test]
    fn test_migration_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // Should not error
    }

    #[test]
    fn test_migration_creates_tables_v13() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"prompt_versions".to_string()));
        assert!(tables.contains(&"prompt_experiments".to_string()));
        assert!(tables.contains(&"experiment_variants".to_string()));
        assert!(tables.contains(&"experiment_metrics".to_string()));
    }

    #[test]
    fn test_migrate_v22_adds_user_id_and_channel_columns() {
        // RBAC M1: pre-existing audit_entries rows must keep working after
        // the schema upgrade — both columns must be NULL-able.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "audit_entries", "user_id"));
        assert!(column_exists(&conn, "audit_entries", "channel"));

        // Insert with the legacy column list (omitting user_id/channel) —
        // must succeed with NULLs. This is the path callers using the
        // pre-M1 INSERT signature take.
        conn.execute(
            "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                0_i64,
                "2026-04-26T00:00:00+00:00",
                "agent-1",
                "AgentSpawn",
                "boot",
                "ok",
                "0".repeat(64),
                "deadbeef".repeat(8),
            ],
        )
        .expect("legacy INSERT must still work after v22");

        let (uid, ch): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT user_id, channel FROM audit_entries WHERE seq = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(uid, None);
        assert_eq!(ch, None);
    }

    #[test]
    fn test_migrate_v22_preserves_existing_rows() {
        // Simulate an upgrade from v21: create a v21-shape audit_entries
        // table by hand, drop in a row, then run migrations. The row must
        // survive intact and gain NULL user_id / channel columns.
        let conn = Connection::open_in_memory().unwrap();
        // Run the pre-v22 migrations only by stopping at v21 state.
        // Easiest: run all migrations, drop the column, and re-add via v22
        // logic. But that defeats the test. Instead build the legacy
        // schema explicitly.
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            );
            CREATE TABLE migrations (version INTEGER PRIMARY KEY, applied_at TEXT, description TEXT);
            INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash) \
              VALUES (0, '2026-01-01T00:00:00+00:00', 'agent-1', 'AgentSpawn', 'boot', 'ok', '0', 'h');",
        )
        .unwrap();

        // Apply just the v22 step.
        migrate_v22(&conn).unwrap();

        assert!(column_exists(&conn, "audit_entries", "user_id"));
        assert!(column_exists(&conn, "audit_entries", "channel"));

        // Original row must be intact, with NULL for the new columns.
        let (agent, uid, ch): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT agent_id, user_id, channel FROM audit_entries WHERE seq = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(agent, "agent-1");
        assert_eq!(uid, None);
        assert_eq!(ch, None);
    }

    #[test]
    fn test_migrate_v22_is_idempotent() {
        // Running run_migrations twice on the same DB must be a no-op
        // for the v22 step — `column_exists` guards the ALTER TABLE so
        // re-running does not try to add the same column twice.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Second run on already-v22 schema must succeed.
        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "audit_entries", "user_id"));
        assert!(column_exists(&conn, "audit_entries", "channel"));
        // Schema version stays at the latest.
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn test_migrate_v23_adds_user_id_and_channel_to_usage_events() {
        // RBAC M5: usage_events gains NULL-able user_id / channel columns
        // for per-user spend rollup. Pre-M5 INSERTs (no user_id/channel in
        // the column list) must keep working with NULL values.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "usage_events", "user_id"));
        assert!(column_exists(&conn, "usage_events", "channel"));

        // Pre-M5 INSERT path — must still work, columns default to NULL.
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, input_tokens, output_tokens, cost_usd, tool_calls) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "u1",
                "agent-1",
                "2026-04-26T00:00:00+00:00",
                "claude-haiku",
                100_i64,
                50_i64,
                0.001_f64,
                0_i64,
            ],
        )
        .expect("legacy INSERT must still work after v23");

        let (uid, ch): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT user_id, channel FROM usage_events WHERE id = 'u1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(uid, None);
        assert_eq!(ch, None);
    }

    #[test]
    fn test_migrate_v23_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "usage_events", "user_id"));
        assert!(column_exists(&conn, "usage_events", "channel"));
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }
}
