//! SQLite schema creation and migration.
//!
//! Creates all tables needed by the memory substrate on first boot.

use rusqlite::Connection;

/// Current schema version.
const SCHEMA_VERSION: u32 = 37;

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
    // v32 (#4496, merged): denormalized `sessions.message_count` for
    // `list_sessions` performance.
    run_step!(32, migrate_v32);
    // v33 (this branch, #3548): rebuild sessions_fts with explicit
    // unicode61 tokenizer + backfill any sessions missing FTS rows.
    run_step!(33, migrate_v33);
    // v34 (#3637): persistent Idempotency-Key cache for state-creating
    // POSTs. The API layer reads/writes this table via the substrate
    // connection pool (handed out via `MemorySubstrate::pool()`).
    run_step!(34, migrate_v34);
    // v35 (#3313): add `tool_use_id` column to `pending_approvals` so
    // the LLM-assigned tool_use id survives a daemon restart and the
    // ACP adapter can keep correlating restored approvals back to the
    // streaming `ToolCall` card.
    run_step!(35, migrate_v35);
    // v36 (#3313): persist the full `DeferredToolExecution` payload so
    // a tool waiting for approval can resume after a daemon restart.
    // Pre-v36 rows have `NULL` here so restored entries still bypass
    // the deferred-execution spawn (matching the prior in-memory
    // behaviour exactly).
    run_step!(36, migrate_v36);
    // v37 (#3335): workflow run persistence in SQLite. Replaces the
    // tmp+rename JSON file (`workflow_runs.json`) that lost Running/Pending
    // state on any shutdown and didn't survive power loss.
    run_step!(37, migrate_v37);

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

        -- Session history.
        --
        -- `message_count` is a denormalised mirror of `len(rmp_serde::decode(messages))`
        -- maintained by `save_session`. It exists so `list_sessions` (and the
        -- per-agent variant) can render a count column without deserialising
        -- every potentially MB-sized blob (#3607). The column is added on the
        -- v1 CREATE TABLE for fresh installs; existing databases gain it via
        -- migration v32, which also backfills `message_count` from the blob
        -- one row at a time.
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            messages BLOB NOT NULL,
            context_window_tokens INTEGER DEFAULT 0,
            message_count INTEGER NOT NULL DEFAULT 0,
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

/// Version 32: Add `message_count` column to `sessions` and backfill it (#3607).
///
/// Pre-v32, `list_sessions()` deserialised every session's full `messages`
/// MessagePack blob solely to populate the `message_count` field in API
/// responses. With many sessions per agent (a 100-agent x 10-session system
/// is typical) that's a thousand multi-MB deserialisations per dashboard
/// page load.
///
/// The fix is a redundant `message_count` column kept in sync inside
/// `save_session()`. Because the writer maintains the invariant from now
/// on, `list_sessions()` can read it directly with no blob round-trip.
///
/// Backfill walks every existing row, decodes the blob once, and writes
/// the count. Rows that fail to decode (corrupt or empty blobs) are left
/// at the column default of `0` and a warning is logged — that matches
/// the pre-fix behaviour where `unwrap_or_default()` produced an empty
/// `Vec<Message>` and a count of `0`. Each row commits in its own
/// statement so the migration's memory footprint is bounded by the
/// largest single blob, not the whole table.
fn migrate_v32(conn: &Connection) -> Result<(), rusqlite::Error> {
    // 1. Add the column. NOT NULL with a literal default is permitted by
    //    SQLite for `ALTER TABLE ... ADD COLUMN`, so existing rows
    //    immediately satisfy the constraint at `0`.
    if !column_exists(conn, "sessions", "message_count") {
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN message_count INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    // 2. Backfill: stream rows one at a time so a database with thousands
    //    of large blobs doesn't pin everything in RAM at once. We use a
    //    fresh prepared statement scope so the read borrow on `conn` is
    //    dropped before we issue the per-row UPDATE statements (rusqlite
    //    forbids holding a `Statement` and calling `execute` on the same
    //    `Connection` simultaneously).
    let mut to_update: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, messages FROM sessions WHERE message_count = 0 AND LENGTH(messages) > 0",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })?;
        for row in rows {
            to_update.push(row?);
        }
    }

    let mut decoded_ok: u64 = 0;
    let mut decoded_err: u64 = 0;
    for (id, blob) in to_update {
        // Decode just enough to count entries. We use the same deserialiser
        // that `save_session`/`get_session` use, so a row that cannot be
        // counted here cannot be loaded as a session either — leaving
        // `message_count = 0` for those rows preserves the pre-fix
        // observable behaviour (`unwrap_or_default()` produced len = 0).
        match rmp_serde::from_slice::<Vec<librefang_types::message::Message>>(&blob) {
            Ok(messages) => {
                let n = messages.len() as i64;
                conn.execute(
                    "UPDATE sessions SET message_count = ?1 WHERE id = ?2",
                    rusqlite::params![n, id],
                )?;
                decoded_ok += 1;
            }
            Err(e) => {
                decoded_err += 1;
                tracing::warn!(
                    session_id = %id,
                    error = %e,
                    "v32 backfill: could not decode messages blob; leaving message_count = 0",
                );
            }
        }
    }

    if decoded_ok > 0 || decoded_err > 0 {
        tracing::info!(
            backfilled = decoded_ok,
            skipped = decoded_err,
            "v32 backfill: populated sessions.message_count from existing blobs (#3607)",
        );
    }

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (32, datetime('now'), 'Add message_count column to sessions and backfill from blob (#3607)')",
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

/// Version 33: Harden `sessions_fts` (issue #3548).
///
/// Recreates the FTS5 virtual table with an explicit
/// `tokenize='unicode61'` so the at-insert tokenization path is
/// documented, stable, and matches what query-side normalization
/// (`SessionStore::search_sessions_paginated`) assumes. The previous
/// schema (migration v12) relied on the implicit default tokenizer,
/// which is `unicode61` today but is a deployment-environment
/// implicit and must not be left implicit in the schema definition.
///
/// **Content preservation.** SQLite has no ALTER for FTS tokenize
/// options, so a DROP+CREATE is the only path to make the tokenizer
/// explicit. Naively dropping wipes every existing FTS row, which
/// silently kills full-text search for any session that isn't saved
/// again post-upgrade (inactive / archived sessions are the worst
/// case — users would just observe "search no longer finds my old
/// chats"). To avoid that we snapshot the existing rows into a temp
/// table, recreate `sessions_fts` with the explicit tokenizer, and
/// re-insert the snapshot. FTS5 re-tokenizes on insert, so the
/// rebuilt index is byte-identical when the old default already was
/// `unicode61` (true for current SQLite builds) and self-heals
/// otherwise.
///
/// **Backfill.** After the snapshot is restored, any row in `sessions`
/// that *still* has no matching FTS row (pre-v12 sessions, drift from
/// #3451-era partial writes) gets an empty placeholder inserted so it
/// is at least visible to the index. The placeholder is overwritten
/// with the real text on the next `save_session` for that session;
/// reflowing it during the migration would require decoding the
/// rmp_serde-encoded `messages` blob and running
/// `SessionStore::extract_text_content` here, which we judged not
/// worth the migration-time cost when the placeholder + lazy reflow
/// already covers any session a user actually interacts with.
///
/// We keep the `(session_id, agent_id, content)` column shape from v12
/// — `search_sessions` filters on `agent_id`, `delete_session` /
/// `execute_session_agent_deletes` look it up by `session_id`, and the
/// boot reconcile reads `session_id` directly. Switching to a
/// content-linked (`content='sessions'`) layout would require
/// rewriting all four call sites and the SQL above; the explicit
/// tokenizer + transactional sync covered by `save_session` and
/// `delete_session` already address the failure modes called out in
/// #3548 (double-index after soft-delete-then-recreate, swallowed
/// errors, pre-v12 invisibility). Schema choice is documented here so
/// a later PR can revisit if the cost of app-level sync becomes a
/// hotspot.
fn migrate_v33(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Atomicity comes from the outer `run_step!` transaction (or, in
    // tests, the caller); SQLite forbids nested transactions, so this
    // body uses bare statements and relies on that wrapper to roll
    // back the whole rebuild on failure.
    //
    // Snapshot existing FTS rows so the DROP+CREATE doesn't silently
    // wipe searchable history. The temp table is a regular SQL table
    // (not FTS5) — we only need the raw column values to re-insert
    // them under the new tokenizer.
    conn.execute_batch(
        "DROP TABLE IF EXISTS _sessions_fts_pre_v33;
         CREATE TEMP TABLE _sessions_fts_pre_v33 (
             session_id TEXT,
             agent_id   TEXT,
             content    TEXT
         );
         INSERT INTO _sessions_fts_pre_v33 (session_id, agent_id, content)
             SELECT session_id, agent_id, content FROM sessions_fts;
         DROP TABLE sessions_fts;
         CREATE VIRTUAL TABLE sessions_fts USING fts5(
             session_id UNINDEXED,
             agent_id   UNINDEXED,
             content,
             tokenize = 'unicode61'
         );
         INSERT INTO sessions_fts (session_id, agent_id, content)
             SELECT session_id, agent_id, content FROM _sessions_fts_pre_v33;
         DROP TABLE _sessions_fts_pre_v33;",
    )?;

    // Backfill: surface every session that still has no FTS row (pre-v12
    // entries, partial-write drift) as an empty placeholder so it stays
    // visible to the index. The next `save_session` call for that session
    // overwrites `content` with the freshly extracted text inside the
    // same transaction as the parent INSERT.
    conn.execute(
        "INSERT INTO sessions_fts (session_id, agent_id, content) \
         SELECT id, agent_id, '' FROM sessions \
         WHERE id NOT IN (SELECT session_id FROM sessions_fts)",
        [],
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (33, datetime('now'), 'Rebuild sessions_fts with explicit unicode61 tokenizer + content-preserving backfill (#3548)')",
        [],
    )?;
    Ok(())
}

/// Test-only re-export so `librefang_memory::session::tests` can drive
/// `migrate_v33` directly when simulating pre-v33 / pre-v12 drift.
/// Production callers go through `run_migrations`.
#[cfg(test)]
pub(crate) fn __test_only_run_v33(conn: &Connection) {
    migrate_v33(conn).expect("migrate_v33 in test harness must succeed");
}

/// Version 34: Persistent Idempotency-Key cache for state-creating POSTs (#3637).
///
/// Adds the `idempotency_keys` table consumed by the API layer's
/// idempotency middleware. Requests that opt in via the
/// `Idempotency-Key` HTTP header have their (status, response body)
/// snapshot persisted here so a duplicate request — same key, same
/// body — replays the prior response instead of re-executing the
/// handler. A duplicate request with the same key but a *different*
/// body produces 409 Conflict; the cache key is the operator-supplied
/// `Idempotency-Key`, while `body_hash` (sha256 of the canonical JSON
/// bytes) is the conflict detector.
///
/// Window: 24 hours. `expires_at = created_at + 86400`. Long enough
/// to absorb realistic webhook / dashboard double-submit windows
/// without retaining replayable state indefinitely. Expired rows are
/// reclaimed lazily on read (the API layer's `prune_expired` hook).
fn migrate_v34(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS idempotency_keys (
             key             TEXT PRIMARY KEY,
             body_hash       TEXT NOT NULL,
             response_status INTEGER NOT NULL,
             response_body   BLOB NOT NULL,
             created_at      INTEGER NOT NULL,
             expires_at      INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_idempotency_keys_expires_at
             ON idempotency_keys(expires_at);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (34, datetime('now'), 'Add idempotency_keys table for Idempotency-Key replay cache (#3637)')",
        [],
    )?;
    Ok(())
}

/// Version 35: Add `tool_use_id` column to `pending_approvals` so the
/// LLM-assigned tool_use id survives a daemon restart (#3313).
///
/// The ACP adapter uses `tool_use_id` to correlate the editor's
/// permission modal with the streaming `ToolCall` card the editor
/// already rendered. Without persistence, restored approvals from a
/// pre-restart session lose the id and fall back to a clearly-namespaced
/// `approval-{req_id}` ToolCallId — functional but visually
/// disconnected.
fn migrate_v35(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch("ALTER TABLE pending_approvals ADD COLUMN tool_use_id TEXT;")?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (35, datetime('now'), 'Add tool_use_id to pending_approvals for ACP adapter cross-restart correlation (#3313)')",
        [],
    )?;
    Ok(())
}

/// Version 36: Persist `DeferredToolExecution` payload so deferred
/// tool runs can resume after a daemon restart (#3313).
///
/// Stored as a `BLOB` containing the JSON serialisation of
/// `librefang_types::tool::DeferredToolExecution`. NULL on rows from
/// the synchronous `request_approval` path or from pre-v36 daemons.
/// On restore, `ApprovalManager::restore_pending_approvals` decodes
/// the blob and rebuilds the `PendingRequest.deferred` slot so a
/// post-restart `Allow once` properly triggers
/// `handle_approval_resolution → execute_deferred_tool`.
fn migrate_v36(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch("ALTER TABLE pending_approvals ADD COLUMN deferred_payload BLOB;")?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (36, datetime('now'), 'Persist DeferredToolExecution on pending_approvals for cross-restart resume (#3313)')",
        [],
    )?;
    Ok(())
}

/// Version 37: Workflow run persistence in SQLite (#3335).
///
/// Replaces the `workflow_runs.json` tmp+rename file with a proper SQLite
/// table. Running and Pending states are now persisted — previous JSON
/// approach filtered them out, losing in-flight work on daemon shutdown
/// or power loss. The `state` CHECK constraint is enforced by the database
/// so invalid values cannot be written by buggy callers.
fn migrate_v37(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_runs (
            id                   TEXT PRIMARY KEY,
            workflow_id          TEXT NOT NULL,
            workflow_name        TEXT NOT NULL DEFAULT '',
            state                TEXT NOT NULL CHECK (state IN ('pending','running','paused','completed','failed')),
            input                TEXT NOT NULL DEFAULT '',
            output               TEXT,
            error                TEXT,
            resume_token         TEXT,
            pause_reason         TEXT,
            paused_at            TEXT,
            paused_step_index    INTEGER,
            paused_variables     TEXT,
            paused_current_input TEXT,
            step_results         TEXT NOT NULL DEFAULT '[]',
            started_at           TEXT NOT NULL,
            completed_at         TEXT,
            created_at           TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_state
            ON workflow_runs(state);
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow_id
            ON workflow_runs(workflow_id);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (37, datetime('now'), 'Add workflow_runs table for SQLite-backed workflow persistence (#3335)')",
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

    /// Issue #3607: v32 adds a `message_count` column on `sessions` and
    /// backfills it from the messages blob so `list_sessions()` can read
    /// the count directly instead of deserialising every blob.
    #[test]
    fn test_migrate_v32_adds_and_backfills_message_count() {
        use librefang_types::message::Message;

        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "sessions", "message_count"));

        // Seed two sessions through the raw INSERT path with a messages
        // blob holding 3 messages, deliberately leaving message_count
        // at the default (0) — this simulates a row written by the
        // pre-v32 writer.
        let agent_id = uuid::Uuid::new_v4().to_string();
        let three: Vec<Message> = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
        ];
        let blob = rmp_serde::to_vec_named(&three).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let sid_a = uuid::Uuid::new_v4().to_string();
        let sid_b = uuid::Uuid::new_v4().to_string();
        for sid in [&sid_a, &sid_b] {
            conn.execute(
                "INSERT INTO sessions \
                   (id, agent_id, messages, context_window_tokens, message_count, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
                rusqlite::params![sid, agent_id, blob, now],
            )
            .unwrap();
        }
        // A third session with an undecodable blob — backfill must not
        // abort the whole migration; that row stays at the default 0.
        let sid_bad = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions \
               (id, agent_id, messages, context_window_tokens, message_count, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
            rusqlite::params![sid_bad, agent_id, vec![0xff_u8, 0xff, 0xff], now],
        )
        .unwrap();

        // Re-run the v32 backfill explicitly. `run_migrations` is a no-op
        // at this point because `user_version` is already at the head, so
        // we drive the backfill directly to assert it works on a
        // pre-populated table.
        migrate_v32(&conn).unwrap();

        let count_a: i64 = conn
            .query_row(
                "SELECT message_count FROM sessions WHERE id = ?1",
                [&sid_a],
                |r| r.get(0),
            )
            .unwrap();
        let count_b: i64 = conn
            .query_row(
                "SELECT message_count FROM sessions WHERE id = ?1",
                [&sid_b],
                |r| r.get(0),
            )
            .unwrap();
        let count_bad: i64 = conn
            .query_row(
                "SELECT message_count FROM sessions WHERE id = ?1",
                [&sid_bad],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_a, 3);
        assert_eq!(count_b, 3);
        assert_eq!(
            count_bad, 0,
            "undecodable blob must leave message_count at the default"
        );
    }

    /// v32 must be idempotent — running it again must not double-count or
    /// re-process rows that already have a non-zero `message_count`.
    #[test]
    fn test_migrate_v32_is_idempotent() {
        use librefang_types::message::Message;

        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let agent_id = uuid::Uuid::new_v4().to_string();
        let two: Vec<Message> = vec![Message::user("x"), Message::assistant("y")];
        let blob = rmp_serde::to_vec_named(&two).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let sid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions \
               (id, agent_id, messages, context_window_tokens, message_count, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
            rusqlite::params![sid, agent_id, blob, now],
        )
        .unwrap();

        migrate_v32(&conn).unwrap();
        let after_first: i64 = conn
            .query_row(
                "SELECT message_count FROM sessions WHERE id = ?1",
                [&sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_first, 2);

        // Second pass must not change anything — the WHERE clause filters
        // out rows with `message_count > 0`, so this row is skipped.
        migrate_v32(&conn).unwrap();
        let after_second: i64 = conn
            .query_row(
                "SELECT message_count FROM sessions WHERE id = ?1",
                [&sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_second, 2);
    }

    /// Issue #3548: v33 must rebuild `sessions_fts` with an explicit
    /// `unicode61` tokenizer. The pragma `table_info` does not expose
    /// FTS5 options, so we instead read `sql` from `sqlite_master` and
    /// assert the literal `tokenize` clause survived. Without it, the
    /// schema continues to depend on the SQLite default, which is
    /// `unicode61` today but is not a contract.
    #[test]
    fn test_migrate_v33_sets_unicode61_tokenizer() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sessions_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("unicode61"),
            "sessions_fts schema must declare unicode61 explicitly; got: {sql}"
        );
        // Sanity: the column shape is preserved so existing call sites
        // that filter on session_id / agent_id keep working.
        assert!(sql.contains("session_id"));
        assert!(sql.contains("agent_id"));
        assert!(sql.contains("content"));
    }

    /// Issue #3548: v33 must backfill an FTS row for every session that
    /// was missing one — pre-v12 sessions, sessions whose write crashed
    /// between the parent INSERT and the FTS sync, etc. Simulates a
    /// pre-v33 state by manually clearing `sessions_fts` after seeding
    /// `sessions`, then re-runs `migrate_v33` and asserts every session
    /// id now has its FTS row.
    #[test]
    fn test_migrate_v33_backfills_missing_fts_rows() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Seed three sessions so the backfill has something to find.
        let agent = uuid::Uuid::new_v4().to_string();
        let ids: Vec<String> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
        for id in &ids {
            conn.execute(
                "INSERT INTO sessions (id, agent_id, messages, context_window_tokens, created_at, updated_at) \
                 VALUES (?1, ?2, x'90', 0, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                rusqlite::params![id, agent],
            )
            .unwrap();
        }

        // Simulate the pre-v33 / pre-v12 drift by emptying sessions_fts
        // entirely, then re-running migrate_v33 directly. The migration
        // body is idempotent: DROP IF EXISTS, CREATE, INSERT...SELECT
        // WHERE NOT IN.
        conn.execute("DELETE FROM sessions_fts", []).unwrap();
        let count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count_before, 0);

        migrate_v33(&conn).expect("v33 must succeed on a drifted sessions_fts");

        let count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            count_after, 3,
            "every session must have a backfilled FTS row"
        );
        for id in &ids {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions_fts WHERE session_id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "session {id} must have exactly one FTS row");
        }
    }

    /// Issue #3548: v33 must NOT lose pre-existing FTS content. The
    /// previous version of this migration did a naive DROP+CREATE that
    /// silently wiped the searchable index for every session that
    /// wasn't re-saved post-upgrade. Test seeds two FTS rows whose
    /// content contains a distinctive needle, runs `migrate_v33`, and
    /// asserts the needle is still findable through the rebuilt
    /// virtual table.
    #[test]
    fn test_migrate_v33_preserves_existing_fts_content() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Drop user_version back to 32 so we can re-run v33 against a
        // pre-populated table (first run_migrations already executed it
        // against an empty sessions_fts, so we need a clean re-entry).
        set_schema_version(&conn, 32).unwrap();

        let agent = uuid::Uuid::new_v4().to_string();
        let session_kept = uuid::Uuid::new_v4().to_string();
        let session_emptied = uuid::Uuid::new_v4().to_string();

        // Seed sessions table so the WHERE NOT IN backfill clause can
        // also be exercised in the same pass.
        for id in [&session_kept, &session_emptied] {
            conn.execute(
                "INSERT INTO sessions (id, agent_id, messages, context_window_tokens, created_at, updated_at) \
                 VALUES (?1, ?2, x'90', 0, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                rusqlite::params![id, agent],
            )
            .unwrap();
        }

        // Pre-populate sessions_fts with real content for one session
        // (snapshot path) and leave the other un-indexed so the
        // backfill path also runs in the same migration.
        conn.execute(
            "INSERT INTO sessions_fts (session_id, agent_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                session_kept,
                agent,
                "preserved needle distinctivewordbeta42",
            ],
        )
        .unwrap();

        // Re-run v33 directly — exercises snapshot+restore + backfill.
        migrate_v33(&conn).expect("v33 rerun must succeed");

        // The pre-existing content survived the rebuild.
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions_fts \
                 WHERE sessions_fts MATCH ?1",
                ["distinctivewordbeta42"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            hits, 1,
            "v33 must preserve pre-existing FTS content for inactive sessions"
        );

        // The previously un-indexed session got an empty placeholder.
        let backfilled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions_fts WHERE session_id = ?1",
                [&session_emptied],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            backfilled, 1,
            "sessions without an FTS row must still get a backfilled placeholder"
        );

        // Tokenizer is still explicit after the rerun.
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sessions_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sql.contains("unicode61"));

        // Temp table from the rebuild is cleaned up.
        let temp_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_temp_master WHERE name = '_sessions_fts_pre_v33'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(temp_left, 0, "v33 must drop its rebuild temp table");
    }

    /// v33 is idempotent: re-running it must not duplicate rows or
    /// fail on the existing virtual table.
    #[test]
    fn test_migrate_v33_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Run a second time — both DROP IF EXISTS and INSERT WHERE NOT IN
        // are guarded.
        migrate_v33(&conn).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
}
