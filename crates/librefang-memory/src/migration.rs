//! SQLite schema creation and migration.
//!
//! Creates all tables needed by the memory substrate on first boot.

use rusqlite::Connection;

/// Current schema version.
const SCHEMA_VERSION: u32 = 32;

/// Run all migrations to bring the database up to date.
pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    let current_version = get_schema_version(conn);

    // Refuse to run if the DB was created by a newer binary. Silently
    // downgrading `user_version` would corrupt v(N+1)+ columns/indexes.
    if current_version > SCHEMA_VERSION {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::CannotOpen,
                extended_code: 0,
            },
            Some(format!(
                "Database schema version {} is newer than this binary supports ({}). \
                 Downgrade is not supported. Use the correct binary version or restore from backup.",
                current_version, SCHEMA_VERSION
            )),
        ));
    }

    macro_rules! run_step {
        ($version:expr, $migrate_fn:expr) => {
            if current_version < $version {
                let tx = conn.unchecked_transaction()?;
                $migrate_fn(&tx)?;
                set_schema_version(&tx, $version)?;
                tx.commit()?;
            }
        };
    }

    run_step!(1, migrate_v1);
    run_step!(2, migrate_v2);
    run_step!(3, migrate_v3);
    run_step!(4, migrate_v4);
    run_step!(5, migrate_v5);
    run_step!(6, migrate_v6);
    run_step!(7, migrate_v7);
    run_step!(8, migrate_v8);
    run_step!(9, migrate_v9);
    run_step!(10, migrate_v10);
    run_step!(11, migrate_v11);
    run_step!(12, migrate_v12);
    run_step!(13, migrate_v13);
    run_step!(14, migrate_v14);
    run_step!(15, migrate_v15);
    run_step!(16, migrate_v16);
    run_step!(17, migrate_v17);
    run_step!(18, migrate_v18);
    run_step!(19, migrate_v19);
    run_step!(20, migrate_v20);
    run_step!(21, migrate_v21);
    run_step!(22, migrate_v22);
    run_step!(23, migrate_v23);
    run_step!(24, migrate_v24);
    run_step!(25, migrate_v25);

    run_step!(26, migrate_v26);
    run_step!(27, migrate_v27);
    run_step!(28, migrate_v28);
    run_step!(29, migrate_v29);
    run_step!(30, migrate_v30);
    run_step!(31, migrate_v31);
    run_step!(32, migrate_v32);

    // Audit-trail consistency (#3538): user_version must match the count
    // of distinct rows in `migrations`. Drift means an earlier migration
    // applied DDL without recording its audit row — operator tooling
    // that lists `SELECT version FROM migrations` then misses those
    // versions silently. Backfill the missing rows in place so a
    // pre-fix DB self-heals on next boot instead of spamming `error!`
    // every restart, and log a single warn line summarising the rescue.
    // Idempotent: a clean DB inserts nothing because every version
    // already has its row.
    let final_version = get_schema_version(conn);
    let mut backfilled: u32 = 0;
    let mut backfill_failed = false;
    for v in 1..=final_version {
        let exists: i64 = match conn.query_row(
            "SELECT COUNT(*) FROM migrations WHERE version = ?1",
            [v],
            |row| row.get(0),
        ) {
            Ok(n) => n,
            Err(e) => {
                tracing::error!(
                    version = v,
                    error = %e,
                    "Migration audit query failed; cannot verify drift for this version"
                );
                backfill_failed = true;
                break;
            }
        };
        if exists == 0 {
            if let Err(e) = conn.execute(
                "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
                 VALUES (?1, datetime('now'), 'audit-row backfill (#3538)')",
                [v],
            ) {
                tracing::error!(
                    version = v,
                    error = %e,
                    "Migration audit backfill failed for this version"
                );
                backfill_failed = true;
                break;
            }
            backfilled += 1;
        }
    }
    if backfilled > 0 && !backfill_failed {
        tracing::warn!(
            user_version = final_version,
            backfilled,
            "Migration audit drift detected and self-healed: inserted \
             missing audit rows for migrations that previously applied DDL \
             without recording their audit row (#3538)"
        );
    }

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
    // Use column_exists guards — identical to the pattern in v6, v14, v15 — so
    // a retry after a partial failure does not error with "column already exists".
    if !column_exists(conn, "entities", "agent_id") {
        conn.execute(
            "ALTER TABLE entities ADD COLUMN agent_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !column_exists(conn, "relations", "agent_id") {
        conn.execute(
            "ALTER TABLE relations ADD COLUMN agent_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_entities_agent ON entities(agent_id);
         CREATE INDEX IF NOT EXISTS idx_relations_agent ON relations(agent_id);
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (10, datetime('now'), 'Add agent_id to entities and relations');",
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
    )?;
    // Audit row (#3538): every applied migration must produce a row in
    // `migrations` so `user_version` and the audit trail stay aligned.
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (13, datetime('now'), 'Add prompt versioning, experiments, variants, metrics tables')",
        [],
    )?;
    Ok(())
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
    // Audit row (#3538): keep migrations table in sync with user_version.
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (17, datetime('now'), 'Persistent approval audit log')",
        [],
    )?;
    Ok(())
}

fn migrate_v18(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS totp_lockout (
            sender_id  TEXT    PRIMARY KEY,
            failures   INTEGER NOT NULL DEFAULT 0,
            locked_at  INTEGER             -- Unix timestamp (seconds) when lockout started, NULL if below threshold
        );",
    )?;
    // Audit row (#3538): keep migrations table in sync with user_version.
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (18, datetime('now'), 'Add totp_lockout table for second-factor brute-force protection')",
        [],
    )?;
    Ok(())
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

/// Version 24: Add `api_key_hash` column to `paired_devices`.
///
/// Each pairing now mints its own bearer token (hashed at rest — current
/// format is unsalted SHA-256 prefixed `$sha256$`, see
/// `password_hash::hash_device_token`; verification dispatches by prefix
/// so any legacy Argon2 hashes from earlier PR revisions also verify).
/// Existing rows from before this migration get an empty hash — those
/// devices must re-pair to obtain a token; until they do, the auth
/// middleware will simply not find a match for any bearer they present.
fn migrate_v24(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "paired_devices", "api_key_hash") {
        conn.execute(
            "ALTER TABLE paired_devices ADD COLUMN api_key_hash TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (24, datetime('now'), 'Add api_key_hash column to paired_devices for per-device bearer tokens')",
        [],
    )?;
    Ok(())
}

/// Version 25: Add `totp_used_codes` table for TOTP replay prevention.
///
/// Stores SHA-256 hashes of recently-used TOTP codes so that a code cannot be
/// reused within the same 30-second window (or the adjacent window). Entries
/// older than 120 seconds are pruned on every successful verification.
fn migrate_v25(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS totp_used_codes (
            code_hash  TEXT    NOT NULL,  -- SHA-256 hex of the raw 6-digit code
            used_at    INTEGER NOT NULL,  -- Unix timestamp (seconds)
            PRIMARY KEY (code_hash)
        );
        CREATE INDEX IF NOT EXISTS idx_totp_used_codes_used_at
            ON totp_used_codes(used_at);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (25, datetime('now'), 'Add totp_used_codes table for TOTP replay prevention')",
        [],
    )?;
    Ok(())
}

/// Version 26: Persistent pending approvals table (issue #3611).
///
/// Stores approval requests that are waiting for human operator action so
/// they survive daemon restarts. On boot the `ApprovalManager` reads this
/// table and re-populates its in-memory DashMap. Rows are deleted when the
/// request is resolved (approved / denied / expired / timed-out).
fn migrate_v26(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pending_approvals (
            id         TEXT    PRIMARY KEY,
            agent_id   TEXT    NOT NULL,
            session_id TEXT,
            tool_name  TEXT    NOT NULL,
            tool_input TEXT    NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            expires_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_pending_approvals_agent
            ON pending_approvals(agent_id);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (26, datetime('now'), 'Add pending_approvals table for cross-restart persistence (issue #3611)')",
        [],
    )?;
    Ok(())
}

/// Version 27: Add `oauth_used_nonces` table for OIDC nonce single-use enforcement.
///
/// OIDC `state` carries a server-signed nonce that the IdP echoes back in the
/// id_token's `nonce` claim.  #3944 added the equality check but never
/// consumed the nonce, so a callback URL captured from browser history /
/// Referer / proxy logs could be replayed against the daemon repeatedly.
/// Hashes of recently-redeemed nonces live here for the duration of the
/// OAuth flow window (default ~15 minutes); prune sweeps anything older
/// than 1 hour to bound the table.
fn migrate_v27(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS oauth_used_nonces (
            nonce_hash  TEXT    NOT NULL,  -- SHA-256 hex of the raw state nonce
            used_at     INTEGER NOT NULL,  -- Unix timestamp (seconds)
            PRIMARY KEY (nonce_hash)
        );
        CREATE INDEX IF NOT EXISTS idx_oauth_used_nonces_used_at
            ON oauth_used_nonces(used_at);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (27, datetime('now'), 'Add oauth_used_nonces table for OIDC nonce single-use enforcement')",
        [],
    )?;
    Ok(())
}

/// Version 28: Add `group_roster` table for cross-channel group membership tracking.
///
/// Tracks which users have been seen in each group chat (channel + chat_id),
/// persisting across daemon restarts. Agents query this to give names to
/// `@mention`s and to render structured "who's in this room" context.
/// Owned by `RosterStore` in `librefang-memory`.
fn migrate_v28(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS group_roster (
            channel_type TEXT    NOT NULL,
            chat_id      TEXT    NOT NULL,
            user_id      TEXT    NOT NULL,
            display_name TEXT    NOT NULL,
            username     TEXT,
            first_seen   INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            last_seen    INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            PRIMARY KEY (channel_type, chat_id, user_id)
        );",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (28, datetime('now'), 'Add group_roster table for cross-channel group membership tracking')",
        [],
    )?;
    Ok(())
}

/// Version 29: Retention timestamps for soft-deleted memories and finished tasks.
///
/// Adds two unix-epoch timestamp columns so the periodic prune sweeps in
/// `kernel/background_agents` can identify rows ready for hard delete:
/// - `memories.deleted_at` is stamped when a row is soft-deleted (`deleted = 1`).
///   Without this, the embedding BLOB hangs around forever (#3467).
/// - `task_queue.finished_at` is stamped when a row reaches `completed`/`failed`.
///   Without this, the queue grows unbounded (#3466).
///
/// Both columns are nullable: pre-migration soft-deletes / completions get
/// NULL and are treated as "not yet eligible for hard delete" by the sweep,
/// which compares `< (now - retention_days)`.
fn migrate_v29(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "memories", "deleted_at") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN deleted_at INTEGER DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memories_deleted_at \
         ON memories(deleted, deleted_at)",
        [],
    )?;
    if !column_exists(conn, "task_queue", "finished_at") {
        conn.execute(
            "ALTER TABLE task_queue ADD COLUMN finished_at INTEGER DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_task_queue_finished_at \
         ON task_queue(status, finished_at)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (29, datetime('now'), 'Add deleted_at/finished_at retention timestamps')",
        [],
    )?;
    Ok(())
}

/// Version 30: Add `session_id` column to `usage_events` so spend/tokens can
/// be rolled up per session (Recent sessions table on the dashboard).
/// Pre-v30 rows leave `session_id` NULL and are simply excluded from
/// per-session aggregates.
fn migrate_v30(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "usage_events", "session_id") {
        conn.execute("ALTER TABLE usage_events ADD COLUMN session_id TEXT", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_events(session_id)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (30, datetime('now'), 'Add session_id column to usage_events for per-session cost rollup')",
        [],
    )?;
    Ok(())
}

/// Version 31: Bind TOTP used codes to the action they authorized (#3360).
///
/// Adds a nullable `bound_to` column on `totp_used_codes` so an auditor can
/// prove which action a given TOTP code authorized (e.g.
/// `"approval:<uuid>"`). Replay detection itself is unchanged — it still
/// keys on `code_hash` so a code is single-use across all actions.
fn migrate_v31(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "totp_used_codes", "bound_to") {
        conn.execute_batch("ALTER TABLE totp_used_codes ADD COLUMN bound_to TEXT;")?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (31, datetime('now'), 'Bind totp_used_codes to the action they authorized (#3360)')",
        [],
    )?;
    Ok(())
}

/// Version 32: Rebuild `sessions_fts` with an explicit tokenizer, add a
/// trigger to cascade-delete on `sessions` removal, and backfill rows for
/// pre-v12 sessions (#3548).
///
/// Three concrete defects this addresses:
///
/// 1. **Tokenizer drift.** v12 created `sessions_fts` without naming a
///    tokenizer. SQLite defaults to `unicode61`, but the version-and-build-
///    dependent default is implicit — querying via `escape_fts5` on a
///    different SQLite build risked subtle case-fold / diacritic
///    differences. Lock the tokenizer to `'unicode61 remove_diacritics 2'`
///    so the index and the query side agree by contract, not by accident.
///
/// 2. **No backfill.** v12 created the table empty. Sessions that already
///    existed pre-v12 never get FTS rows on upgrade — they are invisible to
///    `search_sessions` forever. We can't deserialize the msgpack
///    `messages` blob from SQL, so the backfill inserts a placeholder row
///    (`content = ''`) for every orphaned session id. The next
///    `save_session` call repopulates `content` from the live message text.
///    This at least makes `search_sessions` aware of the row's existence
///    and stops `reconcile_fts_index` from emitting "missing rows" warnings
///    on every boot.
///
/// 3. **No cascade-delete trigger.** Pre-v32 `delete_session` /
///    `delete_agent_sessions` had to manually `DELETE FROM sessions_fts`,
///    and a swallowed-error path (`if let Err(e) = ...; warn!`) could
///    leave orphan FTS rows after a successful row delete. The
///    `sessions_fts_after_delete` trigger now removes the FTS row inside
///    the same statement that deletes from `sessions`, so the manual
///    cleanup is no longer the only safety net.
///
/// The whole migration is idempotent: `DROP TABLE IF EXISTS` + `CREATE
/// VIRTUAL TABLE` + `CREATE TRIGGER IF NOT EXISTS` can run twice with no
/// effect, and the backfill `WHERE NOT IN` clause only inserts rows that
/// don't already exist.
fn migrate_v32(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        -- Drop the v12 table so we can lock the tokenizer. Schemas of FTS5
        -- virtual tables can't be ALTERed in place; we have to drop and
        -- recreate. The backfill below repopulates the rows the drop
        -- discarded.
        DROP TABLE IF EXISTS sessions_fts;

        -- Recreate with an explicit tokenizer pinned to unicode61 with
        -- diacritic folding. `session_id` and `agent_id` stay UNINDEXED so
        -- they're stored verbatim (used by the WHERE clause and the
        -- snippet caller) without bloating the FTS posting list.
        CREATE VIRTUAL TABLE sessions_fts USING fts5(
            session_id UNINDEXED,
            agent_id UNINDEXED,
            content,
            tokenize = 'unicode61 remove_diacritics 2'
        );

        -- Cascade-delete trigger: when a session row goes away, drop its
        -- FTS twin in the same statement so the manual DELETE in the Rust
        -- layer is no longer the only guarantee. The trigger fires inside
        -- the DELETE statement's implicit transaction, so a failure here
        -- rolls back the parent.
        CREATE TRIGGER IF NOT EXISTS sessions_fts_after_delete
        AFTER DELETE ON sessions
        BEGIN
            DELETE FROM sessions_fts WHERE session_id = OLD.id;
        END;

        -- Backfill placeholder rows for every session that was created
        -- before v12 (or whose FTS twin was lost). `content = ''` so
        -- `search_sessions` finds nothing for the row until the next
        -- `save_session` call rewrites the content. The point of the
        -- placeholder is to silence reconcile_fts_index's `missing N` warn
        -- on every boot and to make the row visible to the new trigger.
        INSERT INTO sessions_fts (session_id, agent_id, content)
        SELECT id, agent_id, ''
        FROM sessions
        WHERE id NOT IN (SELECT session_id FROM sessions_fts);
        ",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (32, datetime('now'), 'Rebuild sessions_fts with explicit tokenizer + trigger + backfill (#3548)')",
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
    fn test_every_migration_records_audit_row() {
        // Regression for #3538: each migration must insert into the
        // `migrations` table so that user_version and the audit trail
        // never drift. The startup check at the end of run_migrations
        // logs an error on drift; this test catches it before merge.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let user_version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT version) FROM migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            user_version as i64, row_count,
            "user_version ({user_version}) != distinct migration audit rows ({row_count})"
        );

        // Every version 1..=user_version must appear in the audit table.
        for v in 1..=user_version {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM migrations WHERE version = ?1",
                    [v],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                exists >= 1,
                "migration v{v} is applied (user_version={user_version}) but has no audit row"
            );
        }
    }

    /// Regression for #3538 follow-up: a DB whose migrations table is
    /// already drifted (some audit rows missing) must self-heal on the
    /// next `run_migrations` call instead of warning forever. Simulates
    /// a pre-fix prod DB by deleting v13/v17/v18 audit rows after
    /// migrate, then re-runs and asserts the rows are back. Idempotent
    /// behaviour: a second run inserts nothing.
    #[test]
    fn test_run_migrations_backfills_drifted_audit_rows() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Simulate the historical drift: v13 / v17 / v18 audit rows
        // missing while user_version is at the current latest.
        for v in [13u32, 17u32, 18u32] {
            conn.execute("DELETE FROM migrations WHERE version = ?1", [v])
                .unwrap();
        }

        // Re-run: migrate_vN bodies do not re-execute (user_version is
        // already at the head), so the only path that can heal the
        // missing rows is the backfill at the end of run_migrations.
        run_migrations(&conn).unwrap();

        for v in [13u32, 17u32, 18u32] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM migrations WHERE version = ?1",
                    [v],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 1,
                "audit row for v{v} should have been backfilled, but found {count}"
            );
        }

        // Idempotent: a second backfill pass adds nothing.
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
            .unwrap();
        run_migrations(&conn).unwrap();
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, after, "second backfill must be a no-op");
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

    #[test]
    fn test_migrate_v24_creates_totp_used_codes() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Table must exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(tables.contains(&"totp_used_codes".to_string()));

        // Can insert and look up a code hash
        conn.execute(
            "INSERT INTO totp_used_codes (code_hash, used_at) VALUES (?1, ?2)",
            rusqlite::params!["abcdef1234", 1000_i64],
        )
        .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM totp_used_codes WHERE code_hash = 'abcdef1234'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_migrate_v24_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    /// Issue #3360: v31 adds the `bound_to` column on `totp_used_codes` so
    /// each consumed TOTP code can be tied to the action it authorized.
    #[test]
    fn test_migrate_v31_adds_bound_to_column() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "totp_used_codes", "bound_to"));

        // Inserting with an explicit binding works.
        conn.execute(
            "INSERT INTO totp_used_codes (code_hash, used_at, bound_to) VALUES (?1, ?2, ?3)",
            rusqlite::params!["deadbeef", 2_000_i64, "approval:abc"],
        )
        .unwrap();
        let bound: String = conn
            .query_row(
                "SELECT bound_to FROM totp_used_codes WHERE code_hash = 'deadbeef'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bound, "approval:abc");
    }

    #[test]
    fn test_migrate_v10_partial_apply_does_not_panic() {
        // #3452 — simulate a DB that crashed mid-v10 with the agent_id columns
        // already added but user_version still at 9.  Re-running migrations
        // must succeed (idempotent ALTER) rather than panic with
        // "duplicate column name: agent_id".
        let conn = Connection::open_in_memory().unwrap();

        // Apply v1..v9 to reach the pre-v10 state.
        macro_rules! step {
            ($v:expr, $f:expr) => {{
                let tx = conn.unchecked_transaction().unwrap();
                $f(&tx).unwrap();
                set_schema_version(&tx, $v).unwrap();
                tx.commit().unwrap();
            }};
        }
        step!(1, migrate_v1);
        step!(2, migrate_v2);
        step!(3, migrate_v3);
        step!(4, migrate_v4);
        step!(5, migrate_v5);
        step!(6, migrate_v6);
        step!(7, migrate_v7);
        step!(8, migrate_v8);
        step!(9, migrate_v9);

        // Manually pre-apply the v10 ALTERs as if the previous run crashed
        // after the schema change but before the version bump.
        conn.execute(
            "ALTER TABLE entities ADD COLUMN agent_id TEXT NOT NULL DEFAULT ''",
            [],
        )
        .unwrap();
        conn.execute(
            "ALTER TABLE relations ADD COLUMN agent_id TEXT NOT NULL DEFAULT ''",
            [],
        )
        .unwrap();
        // user_version is still 9 — the partial-apply scenario.
        assert_eq!(get_schema_version(&conn), 9);

        // Resuming migrations from this state must succeed without
        // "duplicate column name: agent_id".
        run_migrations(&conn).expect("v10 retry on partial-apply DB must not error");
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);

        // Columns are still present and writable.
        assert!(column_exists(&conn, "entities", "agent_id"));
        assert!(column_exists(&conn, "relations", "agent_id"));
    }

    #[test]
    fn test_migrate_v10_only_entities_alter_applied() {
        // #3452 follow-up — also cover the asymmetric crash: entities ALTER
        // landed but relations ALTER didn't.  The per-ALTER `column_exists`
        // guards in migrate_v10 must skip entities and apply relations.
        let conn = Connection::open_in_memory().unwrap();
        macro_rules! step {
            ($v:expr, $f:expr) => {{
                let tx = conn.unchecked_transaction().unwrap();
                $f(&tx).unwrap();
                set_schema_version(&tx, $v).unwrap();
                tx.commit().unwrap();
            }};
        }
        step!(1, migrate_v1);
        step!(2, migrate_v2);
        step!(3, migrate_v3);
        step!(4, migrate_v4);
        step!(5, migrate_v5);
        step!(6, migrate_v6);
        step!(7, migrate_v7);
        step!(8, migrate_v8);
        step!(9, migrate_v9);
        // Only entities ALTER pre-applied; relations ALTER did not run.
        conn.execute(
            "ALTER TABLE entities ADD COLUMN agent_id TEXT NOT NULL DEFAULT ''",
            [],
        )
        .unwrap();
        assert!(column_exists(&conn, "entities", "agent_id"));
        assert!(!column_exists(&conn, "relations", "agent_id"));

        run_migrations(&conn).expect("v10 must skip entities ALTER and apply relations ALTER");
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
        assert!(column_exists(&conn, "entities", "agent_id"));
        assert!(column_exists(&conn, "relations", "agent_id"));
    }

    /// Issue #3548: v32 must locks the FTS5 tokenizer to a known value so
    /// the query-side normalization in `escape_fts5` matches the
    /// index-side tokenizer. Verify the table was rebuilt with
    /// `tokenize='unicode61 remove_diacritics 2'` by inspecting
    /// `sqlite_master.sql`.
    #[test]
    fn test_migrate_v32_locks_fts_tokenizer() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='sessions_fts'",
                [],
                |r| r.get(0),
            )
            .expect("sessions_fts must exist after migrations");
        assert!(
            sql.contains("unicode61"),
            "v32 must pin tokenizer to unicode61, got SQL: {sql}"
        );
        assert!(
            sql.contains("remove_diacritics"),
            "v32 must enable remove_diacritics, got SQL: {sql}"
        );
    }

    /// v32 must install an `AFTER DELETE ON sessions` trigger so the FTS
    /// twin row is removed inside the same statement that deletes the
    /// session row, eliminating the swallowed-error path in
    /// `delete_session`.
    #[test]
    fn test_migrate_v32_installs_after_delete_trigger() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let triggers: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='trigger'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            triggers.contains(&"sessions_fts_after_delete".to_string()),
            "v32 must register the cascade-delete trigger; got triggers: {triggers:?}"
        );

        // Functional check: deleting from `sessions` must remove the
        // matching row from `sessions_fts` automatically.
        conn.execute(
            "INSERT INTO sessions (id, agent_id, messages, context_window_tokens, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 0, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            rusqlite::params!["sess-trig", "agent-trig", Vec::<u8>::new()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions_fts (session_id, agent_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params!["sess-trig", "agent-trig", "hello"],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params!["sess-trig"],
        )
        .unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions_fts WHERE session_id = ?1",
                rusqlite::params!["sess-trig"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            remaining, 0,
            "trigger must cascade-delete the FTS twin when the session row is removed"
        );
    }

    /// v32 must backfill placeholder rows for every session that lacks a
    /// `sessions_fts` twin (i.e. pre-v12 sessions). The placeholder
    /// content is empty; future `save_session` calls repopulate it.
    /// This test simulates the upgrade by stopping at v11 (pre-FTS),
    /// inserting a session row, then running the rest of the
    /// migrations (which include v12 + v32).
    #[test]
    fn test_migrate_v32_backfills_pre_v12_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        // Step manually through v1..v11 to land in a pre-FTS state.
        macro_rules! step {
            ($v:expr, $f:expr) => {{
                let tx = conn.unchecked_transaction().unwrap();
                $f(&tx).unwrap();
                set_schema_version(&tx, $v).unwrap();
                tx.commit().unwrap();
            }};
        }
        step!(1, migrate_v1);
        step!(2, migrate_v2);
        step!(3, migrate_v3);
        step!(4, migrate_v4);
        step!(5, migrate_v5);
        step!(6, migrate_v6);
        step!(7, migrate_v7);
        step!(8, migrate_v8);
        step!(9, migrate_v9);
        step!(10, migrate_v10);
        step!(11, migrate_v11);

        // Insert two sessions while no FTS twin exists.
        for sid in ["pre-v12-a", "pre-v12-b"] {
            conn.execute(
                "INSERT INTO sessions (id, agent_id, messages, context_window_tokens, created_at, updated_at) \
                 VALUES (?1, 'agent-bf', ?2, 0, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                rusqlite::params![sid, Vec::<u8>::new()],
            )
            .unwrap();
        }

        // Bring the DB up to head — runs v12 (creates table empty) and v32
        // (rebuilds + backfills).
        run_migrations(&conn).unwrap();
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);

        // Both pre-v12 rows must now have FTS twins (with empty content).
        for sid in ["pre-v12-a", "pre-v12-b"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions_fts WHERE session_id = ?1",
                    rusqlite::params![sid],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 1,
                "v32 backfill must create an FTS twin for pre-v12 session {sid}"
            );
        }

        // No duplicate rows: backfill must not double-insert when a twin
        // already exists.
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 2, "backfill inserts exactly one twin per session");
    }

    /// v32 must be idempotent — running migrations a second time on a
    /// head-version DB must not re-execute the DROP/CREATE (which would
    /// erase live FTS content) or duplicate the backfill.
    #[test]
    fn test_migrate_v32_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Seed a session + its FTS twin.
        conn.execute(
            "INSERT INTO sessions (id, agent_id, messages, context_window_tokens, created_at, updated_at) \
             VALUES ('idem', 'agent-idem', ?1, 0, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            rusqlite::params![Vec::<u8>::new()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions_fts (session_id, agent_id, content) VALUES ('idem', 'agent-idem', 'precious content')",
            [],
        )
        .unwrap();

        // Re-run migrations — must not touch the live row because
        // user_version is already at SCHEMA_VERSION (the run_step macro
        // skips when current >= target).
        run_migrations(&conn).unwrap();

        let preserved: String = conn
            .query_row(
                "SELECT content FROM sessions_fts WHERE session_id = 'idem'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            preserved, "precious content",
            "second migration pass must not drop and rebuild sessions_fts"
        );
    }
}
