//! SQLite schema creation and migration.
//!
//! Creates all tables needed by the memory substrate on first boot.

use rusqlite::Connection;

/// Current schema version.
const SCHEMA_VERSION: u32 = 54;

/// Run all migrations to bring the database up to date.
pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    let current_version = get_schema_version(conn)?;

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

    // Boot-time ladder invariant: `MAX(migrations.version)` must not
    // exceed `pragma user_version`. Each `migrate_vN` runs in its own
    // transaction that bundles the DDL, the `INSERT INTO migrations`
    // audit row, and the `set_schema_version` pragma bump — so under
    // normal operation a mid-ladder crash rolls everything back atomically
    // and the two stay in sync. Drift in the *opposite* direction
    // (audit row present, pragma stuck behind) can still happen via:
    //   * a refactor that moves `INSERT INTO migrations` or the pragma
    //     update outside the per-step tx,
    //   * manual operator surgery on one and not the other,
    //   * two binaries racing on the same DB file.
    // When that drift exists, the run_step! loop below would start from
    // the wrong base (`current_version = user_version`) and silently
    // re-apply DDL whose audit row already exists, corrupting subsequent
    // ALTER TABLEs. Detect it here, before any per-step DDL runs.
    //
    // The opposite direction (`MAX(migrations) < user_version`) is the
    // pre-#3538 audit-drift case, and is healed by the backfill at the
    // end of this function — do not fail on it here.
    //
    // Skip this check on a fresh DB (`user_version == 0`): the
    // `migrations` table itself is created by `migrate_v1`, so it does
    // not yet exist on the very first boot.
    //
    // Operator recovery on `InconsistentLadder`: pick one side as
    // canonical, then realign the other.
    //   * If the live schema matches `table_max`:
    //       `PRAGMA user_version = <table_max>;`
    //   * If the live schema matches `pragma_user_version`:
    //       `DELETE FROM migrations WHERE version > <pragma_user_version>;`
    // Take a backup first; an incorrect choice will mis-route subsequent
    // ALTER TABLEs.
    if current_version > 0 {
        let migrations_table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='migrations'",
            [],
            |row| row.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if migrations_table_exists {
            let table_max: u32 = conn.query_row(
                "SELECT IFNULL(MAX(version), 0) FROM migrations",
                [],
                |row| row.get(0),
            )?;
            if table_max > current_version {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error {
                        code: rusqlite::ffi::ErrorCode::DatabaseCorrupt,
                        extended_code: 0,
                    },
                    Some(format!(
                        "InconsistentLadder: migrations audit table reports \
                         MAX(version)={table_max} but pragma user_version={current_version}. \
                         Refusing to apply migrations from an inconsistent base. \
                         Recovery: if live schema matches version {table_max}, run \
                         `PRAGMA user_version = {table_max};`. If it matches version \
                         {current_version}, run `DELETE FROM migrations WHERE version > {current_version};`. \
                         Back up the database first."
                    )),
                ));
            }
        }
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
    // v38 (#4874): backfill `approval_audit.second_factor_used` for DBs
    // that crossed v17 *before* the TOTP-second-factor patch (#2131)
    // mutated migrate_v17 in place. Those DBs already report
    // user_version >= 17, so neither the v17 CREATE TABLE nor the
    // in-place ALTER inside v17 ever runs again — the column stays
    // missing, every approval-audit INSERT fails, and the user never
    // sees the approval card.
    run_step!(38, migrate_v38);
    // v39 (#4898): per-session model override. Adds `model_override TEXT`
    // to `sessions` so the dashboard chat picker can pin a model to one
    // session without touching the agent manifest. NULL means "use the
    // agent default" (backwards-compatible with all existing rows).
    run_step!(39, migrate_v39);
    // v40 (#5138): fold the `agents.session_id` / `agents.identity` /
    // `agents.source_toml_path` columns into the migration ladder. They
    // were previously bolted on by three `let _ = ALTER TABLE agents ADD
    // COLUMN ...` calls fired on *every* `save_agent`, swallowing the
    // duplicate-column error. That bypassed `user_version` / the
    // `migrations` audit trail entirely and made a refactor that dropped
    // one ALTER silently break fresh installs. Also adds
    // `sessions.messages_generation` so the repair-skip optimisation
    // survives a reload instead of paying a full repair pass every boot.
    run_step!(40, migrate_v40);
    // v41 (audit: sessions-missing-index): the hot path
    // `count_agent_sessions_touched_since` (`SELECT COUNT(*) FROM
    // sessions WHERE agent_id = ?1 AND updated_at > ?2`) and the
    // per-agent cascade `DELETE FROM sessions WHERE agent_id = ?1`
    // had no usable index — the closest was `idx_sessions_peer ON
    // sessions(agent_id, peer_id)` from v16, which works only as a
    // prefix scan and is named in a way that discourages future
    // hinting. On any deployment with > a few thousand sessions
    // both queries degraded to a full-table scan. Add a composite
    // index that the planner picks unambiguously for both
    // `WHERE agent_id = ?` and the `agent_id + updated_at`
    // combined predicate. Also add a composite
    // `audit_entries(agent_id, timestamp)` mirror so the same
    // recency-by-agent shape is fast against the audit trail (v8
    // only created two separate single-column indexes).
    run_step!(41, migrate_v41);
    // v42 (#5744 follow-up): goal run persistence in SQLite. Mirrors the
    // workflow_runs table (v37) so long-horizon goal runs survive a daemon
    // restart instead of vanishing from the in-memory DashMap.
    run_step!(42, migrate_v42);
    // v43 (#6021): mcp_server_configs table for SQLite-backed MCP server config.
    run_step!(43, migrate_v43);
    // v44 (#5981): webauthn_credentials table for passkey (WebAuthn/FIDO2)
    // login. Stores the whole serialized webauthn-rs `Passkey` so the
    // updated sign-count can be persisted after each assertion.
    run_step!(44, migrate_v44);
    // v45 (#5671): channel-instance binding tables backing the deterministic
    // two-level inbound dispatch lookup (instance default + per-conversation
    // override) that replaces the non-deterministic `list_agents().first()`
    // fallback chain.
    run_step!(45, migrate_v45);
    // v46 (#6225): record which session a canonical compaction summary
    // belongs to, so the GET-session banner is shown only on the session
    // whose own history was actually compacted — never leaked onto a
    // freshly created session that merely became the agent's active one.
    run_step!(46, migrate_v46);
    // v47 (#6494): add peer_id to entities and relations so a multi-user
    // agent's knowledge graph is isolated per user, mirroring the
    // memories.peer_id column added in v16. NULL peer_id = shared/unscoped,
    // which is backward compatible with every existing row and a no-op for
    // single-user agents.
    run_step!(47, migrate_v47);
    // v48 (#7756): add `memories.last_decayed_at` so confidence decay charges
    // each interval of idle time exactly once instead of re-applying the whole
    // elapsed idle span on every hourly tick. NULL on pre-migration rows, which
    // the read path treats as "clock starts at accessed_at".
    run_step!(48, migrate_v48);
    // v49 (#7714): add `workflow_runs.owner_agent_id` so a run records which
    // agent asked for it, and `usage_events.billed_agent_id` so a spawned
    // worker's spend rolls up to the agent that spawned it. Both are NULL on
    // every pre-migration row: an ownerless run stays ownerless, and a usage
    // row with no `billed_agent_id` bills to `agent_id` exactly as before.
    run_step!(49, migrate_v49);

    // v50 (#7808): add the `memories_fts` FTS5 index plus the triggers that
    // keep it in step with `memories`, so a deployment with no embedding
    // provider gets real full-text search instead of a `content LIKE` scan —
    // and so `memory.fts_only` finally has a memories index to fall back to.
    run_step!(50, migrate_v50);

    // v51 (#7912): add `memories.embedding_model` so a stored vector records
    // which model produced it. Before this, `memories.embedding` was a bare
    // BLOB: swapping `[memory] embedding_model` between two models of the same
    // dimensionality left every pre-existing vector in the table, and cosine
    // similarity was computed across two unrelated spaces with no error
    // anywhere. NULL means "written before this migration, model unknown" and
    // is treated as comparable, so no existing deployment changes behaviour.
    run_step!(51, migrate_v51);

    // v52 (#7086): add `group_roster.source` so the observational roster and a
    // platform's bulk member list can share one table without sharing one
    // meaning. `channel_dm` authorizes against `source = 'observed'` only.
    //
    // This took 52 rather than 51 because #7916 held 51 while both were in
    // review. #7916 landed first, so the ladder is contiguous and no number is
    // skipped: the audit backfill inserts a row per version in `1..=user_version`,
    // and every one of those versions now has a migration behind it.
    run_step!(52, migrate_v52);

    // v53 (#7752): add the `ephemeral_runs` table so a disposable sub-agent run leaves a record attributable to the agent that spawned it.
    // Before this an ephemeral worker vanished completely — its spend reached the parent's ledger through `usage_events.billed_agent_id` (v49) but the work that produced the spend was invisible, so an operator could not answer "what did this agent delegate, and what did each one cost".
    // Purely additive: no existing table is touched and no existing row changes meaning.
    //
    // Written as v51, renumbered to 53 on merge: #7916 took 51 and #7086 took 52 while this was in review.
    run_step!(53, migrate_v53);

    // v54 (#7930): persist agent lineage.
    // `AgentEntry.parent` had no column at all, so every reload from the store hydrated it as `None` and the API answered `parent_agent_id: null` for agents that demonstrably had a parent.
    // Adds `agents.parent_id` plus the index that makes "list this agent's children" a lookup instead of a full-table scan, and `agents.parent_recorded` so a row written before this migration is distinguishable from an agent that genuinely has no parent.
    //
    // Written against a 51-53 gap held by #7916, #7919 and #7904, which is why it took 54 rather than a contended number. All three have since landed, so the ladder is contiguous and the gap note this comment used to carry no longer describes anything.
    run_step!(54, migrate_v54);

    // Audit-trail consistency (#3538): user_version must match the count
    // of distinct rows in `migrations`. Drift means an earlier migration
    // applied DDL without recording its audit row — operator tooling
    // that lists `SELECT version FROM migrations` then misses those
    // versions silently. Backfill the missing rows in place so a
    // pre-fix DB self-heals on next boot instead of spamming `error!`
    // every restart, and log a single warn line summarising the rescue.
    // Idempotent: a clean DB inserts nothing because every version
    // already has its row.
    let final_version = get_schema_version(conn)?;
    let mut backfilled: u32 = 0;
    let backfill_tx = conn.unchecked_transaction()?;
    for v in 1..=final_version {
        let exists: i64 = match backfill_tx.query_row(
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
                return Err(e);
            }
        };
        if exists == 0 {
            if let Err(e) = backfill_tx.execute(
                "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
                 VALUES (?1, datetime('now'), 'audit-row backfill (#3538)')",
                [v],
            ) {
                tracing::error!(
                    version = v,
                    error = %e,
                    "Migration audit backfill failed for this version"
                );
                return Err(e);
            }
            backfilled += 1;
        }
    }
    backfill_tx.commit()?;
    if backfilled > 0 {
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
fn get_schema_version(conn: &Connection) -> Result<u32, rusqlite::Error> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// Check if a column exists in a table (SQLite has no ADD COLUMN IF NOT EXISTS).
///
/// Schema inspection errors must stop the migration. Treating them as a
/// missing column can route a damaged or locked database into an `ALTER TABLE`
/// branch and obscure the original failure behind a secondary DDL error.
fn try_column_exists(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, rusqlite::Error> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in rows {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    try_column_exists(conn, table, column).expect("test schema inspection must succeed")
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
        if !try_column_exists(conn, "task_queue", name)? {
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
    if !try_column_exists(conn, "memories", "embedding")? {
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
    if !try_column_exists(conn, "sessions", "label")? {
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
    if !try_column_exists(conn, "entities", "agent_id")? {
        conn.execute(
            "ALTER TABLE entities ADD COLUMN agent_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !try_column_exists(conn, "relations", "agent_id")? {
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
    if !try_column_exists(conn, "usage_events", "latency_ms")? {
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
    if !try_column_exists(conn, "memories", "image_url")? {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN image_url TEXT DEFAULT NULL",
            [],
        )?;
    }
    if !try_column_exists(conn, "memories", "image_embedding")? {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN image_embedding BLOB DEFAULT NULL",
            [],
        )?;
    }
    if !try_column_exists(conn, "memories", "modality")? {
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
    if !try_column_exists(conn, "memories", "peer_id")? {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN peer_id TEXT DEFAULT NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memories_peer ON memories(agent_id, peer_id)",
            [],
        )?;
    }
    if !try_column_exists(conn, "sessions", "peer_id")? {
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

/// v47 (#6494): Add peer_id to entities and relations for per-user
/// isolation of the knowledge graph on multi-user agents.
///
/// Mirrors the memories.peer_id column (v16), but entities needs more than a
/// bare column: its `id` is derived from the (lower-cased) entity name, so two
/// users' same-named entities share one `id`. With `id` as the sole PRIMARY
/// KEY they cannot coexist — a second user's entity would collide and be
/// dropped — so the entities table is rebuilt with a composite
/// `PRIMARY KEY (id, peer_id)`, letting each user hold their own row for the
/// same name. relations keeps its single-column `id` PK (a relation row is
/// unique per UUID) and only gains a nullable peer_id used as the read
/// filter.
///
/// The shared/unscoped peer is the empty string `''`, NOT SQL NULL: NULL is
/// distinct from NULL in a UNIQUE/PRIMARY KEY, so `PRIMARY KEY (id, peer_id)`
/// with NULL peers would let the same shared entity insert twice (and the
/// upsert's `ON CONFLICT(id, peer_id)` would never fire), producing duplicate
/// rows that fan out the read JOIN. `''` is a normal value that deduplicates
/// correctly. Every existing row migrates to `peer_id = ''` (shared), so reads
/// without a peer are unchanged and single-user agents are unaffected.
///
/// The caller (`run_step!`) already wraps this in a transaction, so a crash
/// mid-rebuild rolls back and leaves the original entities table intact — this
/// function must NOT open its own transaction (nesting would fail with "cannot
/// start a transaction within a transaction"). Guards make the step idempotent
/// on retry.
fn migrate_v47(conn: &Connection) -> Result<(), rusqlite::Error> {
    // --- entities: rebuild with composite PRIMARY KEY (id, peer_id) ---
    // Skip the rebuild if a prior run already produced the peer_id column
    // (idempotent retry): the new PK is only observable via a rebuild, and the
    // column's presence is our marker that the rebuild completed.
    if !try_column_exists(conn, "entities", "peer_id")? {
        conn.execute_batch(
            "CREATE TABLE entities_new (
                id TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                name TEXT NOT NULL,
                properties TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                agent_id TEXT NOT NULL DEFAULT '',
                peer_id TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (id, peer_id)
            );
            INSERT INTO entities_new (id, entity_type, name, properties, created_at, updated_at, agent_id, peer_id)
                SELECT id, entity_type, name, properties, created_at, updated_at, agent_id, ''
                FROM entities;
            DROP TABLE entities;
            ALTER TABLE entities_new RENAME TO entities;
            CREATE INDEX IF NOT EXISTS idx_entities_agent ON entities(agent_id);
            CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name);
            CREATE INDEX IF NOT EXISTS idx_entities_peer ON entities(agent_id, peer_id);",
        )?;
    }

    // --- relations: additive column, shared sentinel '' (single-column id PK
    // is fine here — a relation row is unique per UUID). ---
    if !try_column_exists(conn, "relations", "peer_id")? {
        conn.execute(
            "ALTER TABLE relations ADD COLUMN peer_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_relations_peer ON relations(agent_id, peer_id)",
            [],
        )?;
    }

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (47, datetime('now'), 'Add peer_id to entities and relations for per-user isolation (#6494)')",
        [],
    )?;
    Ok(())
}

/// v48 (#7756): Record when confidence decay last ran for each memory.
///
/// `decay_confidence` derived its exponent from `accessed_at` and wrote back
/// only `confidence`, so nothing advanced between ticks.
/// Every hourly run therefore re-applied the *entire* elapsed idle span to an
/// already-decayed value, and the accumulated exponent grew quadratically in
/// idle time rather than linearly — a row idle for `D` days accrued
/// `24 * rate * D` per day instead of `rate`.
/// A corpus configured for a ~70-day half-life collapsed to ~1e-3 in weeks.
///
/// `NULL` means "no decay tick has stamped this row yet", which is every row
/// written before this migration.
/// The read path falls back to `accessed_at` for those, so a pre-migration
/// memory takes exactly the one-time decay its doc-commented formula always
/// intended and no row jumps at the migration boundary.
fn migrate_v48(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "memories", "last_decayed_at")? {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN last_decayed_at TEXT DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (48, datetime('now'), 'Add memories.last_decayed_at so confidence decay charges each idle interval once (#7756)')",
        [],
    )?;
    Ok(())
}

/// v51 (#7912): Record which embedding model produced each stored vector.
///
/// `memories.embedding` was a bare BLOB with no provenance, and the only guard
/// against comparing vectors from different models was the length check inside
/// `cosine_similarity`, which returns `None` on a dimension mismatch.
/// Two models of the *same* dimensionality — the issue measured `bge-m3` against
/// `multilingual-e5-large`, both 1024-d — slip straight past that guard, so an
/// operator editing one config line turned every pre-existing row's similarity
/// into a meaningless number with no error and no log line.
///
/// `NULL` means "written before this migration, provenance unknown". Those rows
/// stay comparable: the overwhelmingly common case is that the operator never
/// changed the model, and silently dropping every historical vector out of
/// retrieval would be a far worse default than the risk it removes.
/// The census in `SemanticStore::embedding_model_census` reports them as
/// unstamped so an operator can see how much of their corpus predates the
/// stamp.
///
/// The index exists for the census and for a future re-embedding sweep, both of
/// which group by this column over the whole table.
fn migrate_v51(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "memories", "embedding_model")? {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN embedding_model TEXT DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memories_embedding_model ON memories(embedding_model)",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (51, datetime('now'), 'Add memories.embedding_model so a stored vector records which model produced it (#7912)')",
        [],
    )?;
    Ok(())
}

/// v49 (#7714): Record who owns a workflow run, and who a call's spend bills to.
///
/// `workflow_runs.owner_agent_id` is the agent that asked for the run — the caller of `workflow_run` / `workflow_start`, the agent bound to the channel that issued the command, or the agent an API caller named.
/// It is a property of the *run*, not of the agent that executes a step, which is what lets two owners drive the same shared step-agent type and still keep their attribution apart.
///
/// `usage_events.billed_agent_id` is the agent a call's cost rolls up to: a spawned worker's spend belongs on its spawner's budget line, not on the throwaway child's.
/// It is deliberately a second column rather than a rewrite of `agent_id`: `agent_id` remains the quota subject that both the pre-call `check_quota` and the post-call `check_all_and_record` evaluate, so those two continue to ask about the same agent against that same agent's limits.
/// Folding attribution into `agent_id` would have made the pre-call check read the child's history while the post-call check read the parent's, both against the child's ceiling.
///
/// `NULL` in either column means "no attribution recorded", which is every row written before this migration and every call with no owner or parent.
fn migrate_v49(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "workflow_runs", "owner_agent_id")? {
        conn.execute(
            "ALTER TABLE workflow_runs ADD COLUMN owner_agent_id TEXT DEFAULT NULL",
            [],
        )?;
    }
    if !try_column_exists(conn, "usage_events", "billed_agent_id")? {
        conn.execute(
            "ALTER TABLE usage_events ADD COLUMN billed_agent_id TEXT DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (49, datetime('now'), 'Add workflow_runs.owner_agent_id and usage_events.billed_agent_id for run ownership and spend rollup (#7714)')",
        [],
    )?;
    Ok(())
}
/// v50 (#7808): Full-text index over `memories.content`.
///
/// The schema had exactly one FTS5 table, `sessions_fts`, and nothing over
/// `memories`.
/// Recall with no query embedding therefore fell back to
/// `content LIKE '%{query}%'` — a substring scan that matches nothing unless
/// the caller's entire phrasing appears verbatim inside a memory, which for a
/// natural-language question it essentially never does.
/// The same gap made `memory.fts_only = true` a misnomer for memories: it
/// switches vector search off deliberately, and there was no memories index on
/// the other side of that switch.
///
/// # Shape
///
/// A standalone (non-external-content) FTS5 table mirroring `memories.id` and
/// `content`, matching `sessions_fts`, which the codebase already maintains
/// this way.
/// External-content tables are leaner, but they resolve every hit by rowid
/// against the base table and go silently wrong if the two ever drift; a
/// standalone mirror can be rebuilt from `memories` at any time, and
/// [`rebuild_memories_fts`] does exactly that.
///
/// `agent_id` is carried UNINDEXED so a per-agent search can narrow inside the
/// FTS query rather than filtering after the fact.
/// The tokenizer is declared explicitly for the reason #3548 established on
/// `sessions_fts`: the default is version-dependent, and an index built under
/// one tokenizer answers differently from one built under another.
/// `remove_diacritics 2` folds accents, so `cafe` finds `café`.
///
/// # Sync
///
/// Three triggers mirror INSERT / UPDATE / DELETE on `memories`.
/// Soft deletes (`deleted = 1`) deliberately stay *in* the index: the query
/// path joins back to `memories` and filters `deleted = 0` anyway, and keeping
/// the row means an undelete needs no re-index.
/// The backfill runs unconditionally through [`rebuild_memories_fts`], so
/// re-running this migration on a partially-populated index converges instead
/// of double-inserting.
fn migrate_v50(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            memory_id UNINDEXED,
            agent_id UNINDEXED,
            content,
            tokenize = 'unicode61 remove_diacritics 2'
        );

        CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts (memory_id, agent_id, content)
            VALUES (new.id, new.agent_id, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
            DELETE FROM memories_fts WHERE memory_id = old.id;
        END;

        CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
            DELETE FROM memories_fts WHERE memory_id = old.id;
            INSERT INTO memories_fts (memory_id, agent_id, content)
            VALUES (new.id, new.agent_id, new.content);
        END;
        ",
    )?;

    rebuild_memories_fts(conn)?;

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (50, datetime('now'), 'Add memories_fts FTS5 index over memories.content so search without embeddings is a real index, not a LIKE scan (#7808)')",
        [],
    )?;
    Ok(())
}

/// v52 (#7086): `group_roster.source` — where a roster row came from.
///
/// The roster had exactly one meaning until now: a row existed because that person was observed speaking in that chat.
/// `channel_dm` (#7874) leans on precisely that meaning — it authorizes a private message against the roster, so the set doubles as "people this agent has actually interacted with here".
///
/// The bulk enumeration this issue asked for (Slack's `conversations.members`) reports everyone the platform lists, most of whom have never addressed the agent.
/// Writing those into the same undifferentiated rows would silently promote the whole channel into `channel_dm`'s authorization set — an agent could then DM a member who has never spoken to it, which is a privilege escalation dressed as a feature.
///
/// So the two sets share the table and the primary key but not the predicate.
/// `source` is `'observed'` (someone spoke) or `'enumerated'` (a platform listed them), every pre-migration row is `'observed'` because the sender upsert was the only writer, and the authorization query filters on the column explicitly.
/// A single column beats a second table here because the natural key is identical in both sets: a member who is enumerated and later speaks is one row that changes classification, not two rows needing a UNION with a priority rule at every read site.
fn migrate_v52(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "group_roster", "source")? {
        conn.execute(
            "ALTER TABLE group_roster ADD COLUMN source TEXT NOT NULL DEFAULT 'observed'",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (52, datetime('now'), 'Add group_roster.source so bulk-enumerated members stay out of the channel_dm authorization set (#7086)')",
        [],
    )?;
    Ok(())
}

/// v54 (#7930): persist an agent's parent link.
///
/// `AgentEntry.parent: Option<AgentId>` records which agent spawned another, but the `agents` table never had a column for it.
/// Every hydration site in `structured.rs` therefore hardcoded `parent: None`, and because `routes/agents/mod.rs` serialises the field as `parent_agent_id`, the API positively asserted "this agent has no parent" for every agent after any daemon restart.
/// That is worse than an absent field: `billed_agent_for` in the kernel resolves spend to `entry.parent.unwrap_or(entry.id)`, so a restarted worker silently started billing itself instead of its spawner.
///
/// # Decision 1 — a real column, not a field inside the `manifest` blob
///
/// `parent_id` is a first-class `TEXT` column with `idx_agents_parent_id` over it.
///
/// The manifest blob was the cheaper edit (no DDL, no ladder rung) and it is the wrong home twice over.
/// Structurally, `manifest` is the operator-authored `AgentManifest` — the thing an `agent.toml` round-trips to — whereas parentage is runtime lineage the kernel assigns at spawn.
/// Folding one into the other means `load_all_agents`' auto-repair pass (which re-serialises every manifest it reads and writes back any blob that differs) starts rewriting rows whenever lineage moves, and any future manifest export leaks a runtime edge into a config file.
/// Operationally, a msgpack blob is opaque to SQLite: "list this agent's children" against a blob is a full-table scan that deserialises every manifest in the database, whereas against an indexed column it is `WHERE parent_id = ?1`.
/// That query is the whole reason decision 2 below can be made at all.
///
/// # Decision 2 — `children` is derived, never stored
///
/// There is no `children` column and there will not be one.
/// `children` is a pure inverse of `parent`, and storing both invites the two copies to disagree — which is not hypothetical here: `spawn_agent_inner` already calls `registry.add_child(parent_id, agent_id)` to push the child onto the parent's in-memory entry and then persists only the *child* row, so a stored `children` list would have been born stale on the very first spawn.
/// The read path reconstructs it from `parent_id` instead — a single indexed lookup for `load_agent`, and one in-memory grouping pass for `load_all_agents` — so the relationship has exactly one writer and cannot skew.
///
/// # Decision 3 — `NULL` on a pre-migration row means "unknown", not "root"
///
/// `ALTER TABLE ... ADD COLUMN parent_id` backfills every existing row with `NULL`, and `NULL` is also what a genuine top-level agent stores.
/// Collapsing those two cases would replace one wrong answer with a more confident wrong answer: every agent that predates this migration would begin claiming, positively, to be a root agent.
///
/// So the migration adds a second column, `parent_recorded INTEGER NOT NULL DEFAULT 0`.
/// The `DEFAULT 0` is doing the real work — it is what stamps every pre-existing row as "lineage was never recorded for this row".
/// `save_agent` writes `1` unconditionally from now on, so the three states are distinguishable:
///
/// | `parent_recorded` | `parent_id` | meaning                          |
/// |-------------------|-------------|----------------------------------|
/// | `0`               | `NULL`      | unknown — row predates v54       |
/// | `1`               | `NULL`      | known root — no parent           |
/// | `1`               | id          | child of that agent              |
///
/// A separate boolean beats the cheaper alternative of a sentinel value inside `parent_id` (`''` for "known root", `NULL` for "unknown").
/// A sentinel makes `WHERE parent_id IS NULL` and any future `JOIN agents parent ON a.parent_id = parent.id` quietly wrong for every reader who does not know the convention, and it puts a non-id string in an id column.
/// One byte per agent row is a fair price for a column that describes itself.
///
/// The distinction surfaces on `AgentEntry` as `parent_unknown: bool`.
/// The polarity is deliberate: `Default` / `#[serde(default)]` yields `false` = "recorded", which is correct for every entry the kernel constructs in memory (it knows the lineage it just assigned).
/// Only the store's hydration path can set it `true`, and only for a row that really was written before this migration.
fn migrate_v54(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "agents", "parent_id")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN parent_id TEXT DEFAULT NULL",
            [],
        )?;
    }
    // DEFAULT 0 is the load-bearing part: it marks every row that already existed as "parent was never recorded" (see decision 3 above).
    if !try_column_exists(conn, "agents", "parent_recorded")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN parent_recorded INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    // The index that makes decision 2 (derive `children`) affordable.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_parent_id ON agents(parent_id)",
        [],
    )?;
    // Record the audit row under 54 — this migration's OWN number.
    // `migrate_v50` recorded itself as 49 (#7924/#7925), which desynchronises `user_version` from the `migrations` table and forces the backfill pass at the bottom of `run_migrations` to paper over it on every boot.
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (54, datetime('now'), 'Persist agent lineage: agents.parent_id + idx_agents_parent_id + agents.parent_recorded so a pre-v54 row reads as unknown rather than root (#7930)')",
        [],
    )?;
    Ok(())
}

/// Rebuild `memories_fts` from `memories`, dropping whatever was there.
///
/// Idempotent and safe to call at any time: the index is a pure derivative of
/// `memories`, so a full rebuild is always the correct repair for drift and is
/// what makes [`migrate_v50`] re-runnable.
/// Kept separate from the migration so a future integrity check can reach it
/// without replaying DDL.
fn rebuild_memories_fts(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM memories_fts", [])?;
    conn.execute(
        "INSERT INTO memories_fts (memory_id, agent_id, content) \
         SELECT id, agent_id, content FROM memories",
        [],
    )?;
    Ok(())
}

/// v53 (#7752): ephemeral worker run records.
///
/// An ephemeral worker runs one turn under its parent's identity and then vanishes — no registry entry, no persisted session, and a mission workspace deleted on the way out.
/// That left an operator with nothing to inspect: v49 gave the parent the *spend* through `usage_events.billed_agent_id`, but the work behind the spend had no record at all.
///
/// This table is that record, and it is deliberately not a session.
/// The alternative — clearing the worker's `incognito` flag so the agent loop persists its session under the parent's `AgentId` — also un-gates the episodic-memory write, the context-engine advance and the proactive `auto_memorize` pass, all of which key on the same id, and would fold a delegated sub-run's text into the parent's own recall and conversation list.
/// See `crate::ephemeral_run_store` for the full argument.
///
/// `parent_agent_id` is indexed because every read is scoped to one parent, and carries no foreign key for the same reason the other agent-scoped tables do not: the cascade is explicit, in `MemorySubstrate::remove_agent`, so all agent-scoped deletes stay in one reviewable list rather than split between SQL and Rust.
fn migrate_v53(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ephemeral_runs (
            id              TEXT PRIMARY KEY,
            parent_agent_id TEXT NOT NULL,
            label           TEXT NOT NULL,
            worker_name     TEXT NOT NULL,
            agent_type      TEXT,
            task            TEXT NOT NULL DEFAULT '',
            response        TEXT NOT NULL DEFAULT '',
            status          TEXT NOT NULL CHECK (status IN ('completed','failed')),
            error           TEXT,
            provider        TEXT NOT NULL DEFAULT '',
            model           TEXT NOT NULL DEFAULT '',
            iterations      INTEGER NOT NULL DEFAULT 0,
            tool_calls      INTEGER NOT NULL DEFAULT 0,
            input_tokens    INTEGER NOT NULL DEFAULT 0,
            output_tokens   INTEGER NOT NULL DEFAULT 0,
            cost_usd        REAL NOT NULL DEFAULT 0.0,
            latency_ms      INTEGER NOT NULL DEFAULT 0,
            started_at      TEXT NOT NULL,
            finished_at     TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_ephemeral_runs_parent
            ON ephemeral_runs(parent_agent_id, finished_at DESC);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (53, datetime('now'), 'Add ephemeral_runs table so a disposable sub-agent run leaves a record under its parent (#7752)')",
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
    // `second_factor_used` was added to the CREATE TABLE above by #2131
    // (TOTP second-factor for critical approvals). Fresh installs land
    // here with the column already present from CREATE TABLE; the only
    // case where the table existed without the column is a DB at
    // user_version >= 17 from a pre-#2131 binary — that path is
    // self-healed by `migrate_v38` (issue #4874), not here, because v17
    // does not re-run for those DBs.
    //
    // The previous `let _ = conn.execute("ALTER TABLE ...")` was a
    // dead-letter: it only ran on a fresh install (where the column
    // already existed and the ALTER errored), and *never* ran on the
    // upgrade path where it was actually needed. Removed.
    //
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
    if !try_column_exists(conn, "usage_events", "provider")? {
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
    if !try_column_exists(conn, "task_queue", "claimed_at")? {
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
    if !try_column_exists(conn, "task_queue", "retry_count")? {
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
    if !try_column_exists(conn, "audit_entries", "user_id")? {
        conn.execute("ALTER TABLE audit_entries ADD COLUMN user_id TEXT", [])?;
    }
    if !try_column_exists(conn, "audit_entries", "channel")? {
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
    if !try_column_exists(conn, "usage_events", "user_id")? {
        conn.execute("ALTER TABLE usage_events ADD COLUMN user_id TEXT", [])?;
    }
    if !try_column_exists(conn, "usage_events", "channel")? {
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
    if !try_column_exists(conn, "paired_devices", "api_key_hash")? {
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
    if !try_column_exists(conn, "memories", "deleted_at")? {
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
    if !try_column_exists(conn, "task_queue", "finished_at")? {
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
    if !try_column_exists(conn, "usage_events", "session_id")? {
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
    if !try_column_exists(conn, "sessions", "message_count")? {
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
    if !try_column_exists(conn, "totp_used_codes", "bound_to")? {
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
/// Completed responses use a 24-hour replay window.
/// Pending reservations use the same schema row with status zero, but current store semantics do not expire them automatically while a handler may still own the key.
/// Expired completed rows are reclaimed lazily on read and by the API layer's `prune_expired` hook.
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
            ON workflow_runs(workflow_id);
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_started_at
            ON workflow_runs(started_at DESC);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (37, datetime('now'), 'Add workflow_runs table for SQLite-backed workflow persistence (#3335)')",
        [],
    )?;
    Ok(())
}

/// Version 38: Backfill `approval_audit.second_factor_used` for upgraded DBs (#4874).
///
/// PR #2131 added the `second_factor_used` column by editing
/// `migrate_v17` in place — both the `CREATE TABLE` and an
/// `ALTER TABLE ... ADD COLUMN`. That works for fresh installs
/// (`user_version = 0` → v17 runs → column lands via CREATE TABLE),
/// but for any DB already at `user_version >= 17` from an earlier
/// binary, `migrate_v17` never re-runs, the ALTER never fires, and the
/// column stays missing. Every subsequent `INSERT INTO approval_audit
/// (..., second_factor_used) VALUES (...)` then fails with
/// `table approval_audit has no column named second_factor_used`,
/// the audit-write `warn!` fires, and on the affected installs the
/// approval flow stalls — the user never sees the approval card on
/// any surface (Web, Telegram, `/api/approvals`).
///
/// `column_exists` makes this idempotent: fresh installs (where v17
/// already created the column) and re-runs both no-op cleanly.
fn migrate_v38(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "approval_audit", "second_factor_used")? {
        conn.execute(
            "ALTER TABLE approval_audit ADD COLUMN second_factor_used INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (38, datetime('now'), 'Backfill approval_audit.second_factor_used for upgraded DBs (#4874)')",
        [],
    )?;
    Ok(())
}

/// Version 39: Per-session model override (#4898).
///
/// Adds a nullable `model_override` TEXT column to `sessions`. When set,
/// the kernel's `execute_llm_agent` uses this value instead of the agent
/// manifest's `model.model` / `model.provider` fields for LLM dispatch on
/// that session. `NULL` preserves the existing behaviour (agent default).
///
/// The column stores `"<provider>/<model>"` when a provider is specified,
/// or just `"<model>"` for provider-agnostic overrides. The API layer
/// (`PATCH /api/sessions/{id}/model`) sets and clears it; `NULL` body
/// clears the override and restores the agent default.
fn migrate_v39(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "sessions", "model_override")? {
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN model_override TEXT DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (39, datetime('now'), 'Add model_override column to sessions for per-session model pin (#4898)')",
        [],
    )?;
    Ok(())
}

/// Version 40: Ladder-ize the `agents` schema-evolution columns and add
/// `sessions.messages_generation` (#5138).
///
/// `StructuredStore::save_agent` historically added `session_id`,
/// `identity`, and `source_toml_path` to `agents` via three
/// `let _ = conn.execute("ALTER TABLE agents ADD COLUMN ...")` calls on
/// every save, silencing the duplicate-column error on the steady-state
/// path. Because these columns were never declared in any `migrate_vN`,
/// the `migrations` audit trail and `user_version` never reflected them,
/// and removing one of those ALTERs in a later refactor would silently
/// break new installs that never had the column. This migration declares
/// the columns once, in-ladder, inside the transactional `run_step!`.
///
/// It also adds `sessions.messages_generation INTEGER NOT NULL DEFAULT 0`
/// so the in-memory repair-skip generation counter (`Session::
/// messages_generation`) round-trips across a reload instead of resetting
/// to `0` every cold load — which forced a full repair pass on the first
/// post-load save even when the loaded blob was already repaired
/// (performance only, not correctness).
///
/// `column_exists` keeps every `ADD COLUMN` idempotent: fresh installs
/// (whose v1 CREATE TABLE never carried these) and any DB already carrying
/// the column from the legacy per-save ALTER both no-op cleanly.
/// v41 (audit: sessions-missing-index): composite indexes on
/// `sessions(agent_id, updated_at)` and `audit_entries(agent_id,
/// timestamp)` to keep the per-agent recency hot paths off the
/// full-table-scan plan. Both `CREATE INDEX IF NOT EXISTS` so a
/// fresh boot is idempotent.
fn migrate_v41(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        -- Composite index used by:
        --   - count_agent_sessions_touched_since
        --     (`WHERE agent_id = ?1 AND updated_at > ?2` — concurrent-
        --      trigger admission check, runs on every fire)
        --   - per-agent cascade DELETE on agent removal
        --   - any future ORDER BY updated_at DESC LIMIT N scoped to
        --     one agent (session timeline rendering)
        -- v16 created `idx_sessions_peer ON sessions(agent_id, peer_id)`
        -- which CAN serve `agent_id`-only scans as a prefix but is named
        -- in a way that discourages future hinting; this index is the
        -- explicit, intention-revealing one.
        CREATE INDEX IF NOT EXISTS idx_sessions_agent_updated
            ON sessions(agent_id, updated_at);

        -- Companion composite for the audit trail: v8 created
        -- single-column indexes on agent_id and timestamp, so
        -- per-agent recency queries pick one or the other and pay
        -- a sort. The composite lets the planner stream rows in
        -- (agent_id, timestamp) order without a sort step.
        CREATE INDEX IF NOT EXISTS idx_audit_agent_timestamp
            ON audit_entries(agent_id, timestamp);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (41, datetime('now'), 'sessions(agent_id, updated_at) + audit_entries(agent_id, timestamp) composite indexes (audit: sessions-missing-index)');
        ",
    )
}

/// v42 (#5744 follow-up): goal run persistence in SQLite.
///
/// Long-horizon goal runs (`GoalRunner`) previously kept their live run
/// state in an in-memory DashMap only — a daemon restart lost every active
/// run with no boot recovery, unlike workflow runs (v37). This table is the
/// durable mirror of `GoalRunState`: the runner writes it after each
/// iteration and on stop, and `recover_stale_goal_runs` reads it at boot to
/// demote runs interrupted by a restart.
///
/// `goal_id` is the PRIMARY KEY because at most one run is active per goal
/// (the runner replaces any prior run for the same goal). `phase` is
/// constrained to the `GoalRunPhase` string forms so an unknown value can
/// never round-trip through the table.
fn migrate_v42(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS goal_runs (
            goal_id        TEXT PRIMARY KEY,
            agent_id       TEXT NOT NULL,
            phase          TEXT NOT NULL CHECK (phase IN ('running','finished','max_iterations_reached','rate_limited','stopped')),
            iteration      INTEGER NOT NULL DEFAULT 0,
            max_iterations INTEGER NOT NULL DEFAULT 0,
            last_progress  INTEGER NOT NULL DEFAULT 0,
            last_error     TEXT,
            started_at     TEXT NOT NULL,
            updated_at     TEXT NOT NULL,
            created_at     TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_goal_runs_phase
            ON goal_runs(phase);
        CREATE INDEX IF NOT EXISTS idx_goal_runs_started_at
            ON goal_runs(started_at DESC);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (42, datetime('now'), 'Add goal_runs table for SQLite-backed goal run persistence (#5744)')",
        [],
    )?;
    Ok(())
}

fn migrate_v43(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_server_configs (
            name       TEXT PRIMARY KEY,
            entry_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (43, datetime('now'), 'Add mcp_server_configs table for SQLite-backed MCP server config (#6021)')",
        [],
    )?;
    Ok(())
}

fn migrate_v44(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS webauthn_credentials (
             credential_id TEXT PRIMARY KEY,
             user_name     TEXT NOT NULL,
             cred          TEXT NOT NULL,
             label         TEXT,
             created_at    INTEGER NOT NULL,
             last_used_at  INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_webauthn_credentials_user_name
             ON webauthn_credentials(user_name);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (44, datetime('now'), 'Add webauthn_credentials table for passkey (WebAuthn/FIDO2) login (#5981)')",
        [],
    )?;
    Ok(())
}

fn migrate_v45(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Two tables backing Model A inbound dispatch (#5671):
    //   channel_instance_defaults — one row per `[[sidecar_channels]]`
    //     instance, seeded from config at boot; the default agent a channel
    //     instance routes to when a conversation has no explicit override.
    //   conversation_bindings — per (instance, conversation) override written
    //     by `/agent`; supersedes the instance default. Empty until the
    //     `/agent` command lands, but the read path consults it first.
    // Both store the agent *name* (not the per-spawn `AgentId` uuid): config
    // and the registry resolve agents by stable name, and the bridge maps
    // name -> id at dispatch.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS channel_instance_defaults (
            instance_name TEXT PRIMARY KEY,
            agent_name    TEXT NOT NULL,
            bound_at      TEXT NOT NULL DEFAULT (datetime('now')),
            bound_by      TEXT
        );
        CREATE TABLE IF NOT EXISTS conversation_bindings (
            instance_name   TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            agent_name      TEXT NOT NULL,
            bound_at        TEXT NOT NULL DEFAULT (datetime('now')),
            bound_by        TEXT,
            PRIMARY KEY (instance_name, conversation_id)
        );",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (45, datetime('now'), 'Add channel_instance_defaults + conversation_bindings tables for deterministic inbound dispatch (#5671)')",
        [],
    )?;
    Ok(())
}

/// Version 46: Tag the canonical compaction summary with the session it
/// belongs to (#6225).
///
/// `canonical_sessions.compacted_summary` is agent-scoped (one row per
/// `agent_id`) and outlives any individual session. Before this column the
/// GET-session handler exposed the summary on whichever session happened to
/// be the agent's active one, so creating a brand-new session — which makes
/// it active without ever compacting it — leaked a prior conversation's
/// summary onto message #1. The nullable `compacted_summary_session_id`
/// records which session legitimately owns the current summary; the read
/// path gates the banner on a match. Backward-compatible: existing rows get
/// `NULL`, which the read path treats as "owned by no specific session"
/// (banner hidden) until the next compaction stamps the owning session.
fn migrate_v46(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "canonical_sessions", "compacted_summary_session_id")? {
        conn.execute(
            "ALTER TABLE canonical_sessions ADD COLUMN compacted_summary_session_id TEXT",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (46, datetime('now'), 'Tag canonical compaction summary with owning session_id (#6225)')",
        [],
    )?;
    Ok(())
}

fn migrate_v40(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !try_column_exists(conn, "agents", "session_id")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN session_id TEXT DEFAULT ''",
            [],
        )?;
    }
    if !try_column_exists(conn, "agents", "identity")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN identity TEXT DEFAULT '{}'",
            [],
        )?;
    }
    if !try_column_exists(conn, "agents", "source_toml_path")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN source_toml_path TEXT DEFAULT NULL",
            [],
        )?;
    }
    if !try_column_exists(conn, "sessions", "messages_generation")? {
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN messages_generation INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) \
         VALUES (40, datetime('now'), 'Ladder-ize agents schema columns + add sessions.messages_generation (#5138)')",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn test_schema_version_read_error_stops_before_migration_ddl() {
        let conn = Connection::open_in_memory().unwrap();
        let ddl_attempts = Arc::new(AtomicUsize::new(0));
        let callback_attempts = Arc::clone(&ddl_attempts);

        conn.authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Pragma {
                pragma_name: "user_version",
                ..
            } => Authorization::Deny,
            AuthAction::CreateTable { .. } => {
                callback_attempts.fetch_add(1, Ordering::SeqCst);
                Authorization::Allow
            }
            _ => Authorization::Allow,
        }))
        .unwrap();

        assert!(run_migrations(&conn).is_err());
        assert_eq!(
            ddl_attempts.load(Ordering::SeqCst),
            0,
            "a schema-version read error must fail before migration DDL"
        );
    }

    #[test]
    fn test_final_schema_version_read_error_is_propagated() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let callback_reads = Arc::clone(&reads);

        conn.authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Pragma {
                pragma_name: "user_version",
                pragma_value: None,
            } if callback_reads.fetch_add(1, Ordering::SeqCst) == 1 => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();

        assert!(run_migrations(&conn).is_err());
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

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

        // …and each one must have written that row *itself*.
        // The two checks above run after the #3538 backfill at the end of `run_migrations`, which inserts a placeholder row for any version that failed to record itself — so they pass even when a migration writes its audit row under the wrong version number.
        // `migrate_v50` did exactly that (it recorded itself as 49, where `INSERT OR IGNORE` silently dropped it against v49's existing row), and the only visible symptom was a "drift detected and self-healed" warning on every fresh boot.
        // A clean ladder must need no backfill at all.
        let placeholders: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE description = 'audit-row backfill (#3538)'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            placeholders, 0,
            "a fresh ladder must need no audit-row backfill; some migrate_vN is \
             recording its audit row under a version other than its own"
        );
    }

    /// Companion to `test_every_migration_records_audit_row`, and the guard that test could not be (#7924).
    ///
    /// That test asserts every applied version *has* an audit row, but the #3538 backfill at the end of `run_migrations` creates any missing row before the assertion runs — so a migration that stamps the wrong version number passes it, rescued by the repair path.
    /// `migrate_v50` did exactly that: it recorded under version 49, where `INSERT OR IGNORE` silently dropped it against `migrate_v49`'s row, and every fresh database then warned about historical drift on its first boot.
    ///
    /// The invariant the backfill's own doc comment already claims is the stronger one and is what this pins: on a clean database the backfill inserts nothing, because every migration recorded itself.
    /// A placeholder description surviving a fresh `run_migrations` means some migration is stamping a version that is not its own.
    #[test]
    fn no_migration_relies_on_the_audit_backfill() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT version, description FROM migrations ORDER BY version")
            .unwrap();
        let rescued: Vec<u32> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .filter(|(_, description)| description.contains("audit-row backfill"))
            .map(|(version, _)| version)
            .collect();

        assert!(
            rescued.is_empty(),
            "these versions were rescued by the #3538 backfill on a clean database, which means the \
             migration that applied their DDL stamped a version that is not its own: {rescued:?}"
        );
    }

    /// The specific collision from #7924, pinned by description rather than by presence.
    ///
    /// Both rows existing is not enough — that was already true, because the backfill supplied the missing one.
    /// What has to hold is that each version's description names the DDL that version actually applied.
    #[test]
    fn v49_and_v50_each_describe_their_own_ddl() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let description = |version: u32| -> String {
            conn.query_row(
                "SELECT description FROM migrations WHERE version = ?1",
                [version],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|e| panic!("no audit row for v{version}: {e}"))
        };

        assert!(
            description(49).contains("owner_agent_id"),
            "v49 must describe the run-ownership columns it adds (#7714), got: {}",
            description(49)
        );
        assert!(
            description(50).contains("memories_fts"),
            "v50 must describe the FTS5 index it adds (#7808), not a backfill placeholder or v49's \
             description, got: {}",
            description(50)
        );
    }

    /// Boot-time ladder invariant: a DB whose `migrations` audit table
    /// claims a higher MAX(version) than `pragma user_version` is in an
    /// inconsistent state — `run_migrations` would otherwise restart
    /// from the wrong base and re-apply DDL whose audit row already
    /// exists, silently corrupting later ALTER TABLEs. Simulate the
    /// drift, then assert `run_migrations` refuses to proceed and the
    /// error message names both sides so the operator can recover.
    #[test]
    fn test_run_migrations_rejects_audit_ahead_of_pragma() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Simulate "audit row present, pragma stuck behind": insert a
        // phantom future audit row without bumping user_version. This is
        // what an out-of-tx INSERT (refactor regression) or manual
        // operator surgery on one side would leave behind.
        let phantom_version = SCHEMA_VERSION + 1;
        conn.execute(
            "INSERT INTO migrations (version, applied_at, description) \
             VALUES (?1, datetime('now'), 'phantom test row')",
            [phantom_version],
        )
        .unwrap();

        let err = run_migrations(&conn).expect_err(
            "run_migrations must refuse to proceed when MAX(migrations) > user_version",
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("InconsistentLadder"),
            "error must name the InconsistentLadder condition, got: {msg}"
        );
        assert!(
            msg.contains(&phantom_version.to_string()),
            "error must surface the table_max ({phantom_version}) for operator recovery, got: {msg}"
        );
        assert!(
            msg.contains(&SCHEMA_VERSION.to_string()),
            "error must surface the pragma user_version ({SCHEMA_VERSION}) for operator recovery, got: {msg}"
        );
    }

    /// The ladder guard must fail closed when the audit table exists but its
    /// schema cannot answer `MAX(version)`. Treating that query failure as
    /// zero would bypass the consistency check and let migrations continue
    /// from an unverified base.
    #[test]
    fn test_run_migrations_propagates_audit_table_query_errors() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE migrations (
                 wrong_column INTEGER NOT NULL
             );
             PRAGMA user_version = 1;",
        )
        .unwrap();

        let error = run_migrations(&conn)
            .expect_err("malformed migrations audit table must stop the migration ladder");

        assert!(
            error.to_string().contains("no such column: version"),
            "unexpected error: {error}"
        );
        let user_version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(user_version, 1, "failed guard must not advance the ladder");
    }

    #[test]
    fn test_column_exists_propagates_schema_inspection_errors() {
        let conn = Connection::open_in_memory().unwrap();

        let error = try_column_exists(&conn, "unterminated(", "id")
            .expect_err("invalid schema inspection SQL must not become column-not-found");

        assert!(
            error.to_string().contains("syntax error"),
            "unexpected error: {error}"
        );
    }

    /// Happy path: a freshly migrated DB has `MAX(migrations.version) ==
    /// pragma user_version` and re-running `run_migrations` is a no-op.
    /// Guards against the invariant check itself spuriously tripping on
    /// the common path.
    #[test]
    fn test_run_migrations_accepts_consistent_ladder() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let table_max: u32 = conn
            .query_row("SELECT MAX(version) FROM migrations", [], |row| row.get(0))
            .unwrap();
        let user_version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            table_max, user_version,
            "fresh migrate must leave MAX(migrations)==user_version"
        );

        // Re-running on a consistent ladder must succeed.
        run_migrations(&conn).expect("idempotent re-run on consistent ladder must succeed");
    }

    /// The pre-#3538 drift direction (`MAX(migrations) < user_version`)
    /// is the legacy audit-trail drift that the backfill at the end of
    /// `run_migrations` self-heals. The new invariant must NOT fail on
    /// this direction or it would block every pre-#3538 prod DB from
    /// upgrading. Guards against an over-eager equality check.
    #[test]
    fn test_run_migrations_tolerates_audit_behind_pragma() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Drop the last audit row to simulate the legacy drift.
        conn.execute(
            "DELETE FROM migrations WHERE version = ?1",
            [SCHEMA_VERSION],
        )
        .unwrap();

        // Must NOT error — the backfill heals this direction.
        run_migrations(&conn)
            .expect("MAX(migrations) < user_version is the #3538 self-heal path, not a fatal");

        // Confirm the heal happened.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE version = ?1",
                [SCHEMA_VERSION],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "backfill must restore the deleted audit row");
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
    fn test_run_migrations_fails_and_rolls_back_when_audit_backfill_fails() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        for version in [13u32, 17u32] {
            conn.execute("DELETE FROM migrations WHERE version = ?1", [version])
                .unwrap();
        }
        conn.execute_batch(
            "CREATE TEMP TRIGGER fail_audit_backfill
             BEFORE INSERT ON migrations
             WHEN NEW.version = 17
             BEGIN
                 SELECT RAISE(ABORT, 'forced audit backfill failure');
             END;",
        )
        .unwrap();

        let error = run_migrations(&conn)
            .expect_err("an audit-row insert failure must fail the migration run");

        assert!(
            error.to_string().contains("forced audit backfill failure"),
            "unexpected error: {error}"
        );
        for version in [13u32, 17u32] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM migrations WHERE version = ?1",
                    [version],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 0,
                "failed backfill must roll back every audit row from the transaction"
            );
        }

        conn.execute_batch("DROP TRIGGER fail_audit_backfill;")
            .unwrap();
        run_migrations(&conn).expect("a later healthy run must heal the audit drift");
        for version in [13u32, 17u32] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM migrations WHERE version = ?1",
                    [version],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "retry must restore migration v{version}");
        }
    }

    /// Regression for #4874: a DB that crossed `migrate_v17` on a binary
    /// that pre-dated #2131 has `approval_audit` *without* the
    /// `second_factor_used` column. When the user later upgrades to a
    /// binary whose `migrate_v17` adds that column, v17 never re-runs
    /// (user_version is already past it), so the column stays missing
    /// and every approval-audit INSERT fails. `migrate_v38` must
    /// backfill the column on this path.
    #[test]
    fn test_migrate_v38_backfills_second_factor_used_on_legacy_v17_schema() {
        let conn = Connection::open_in_memory().unwrap();
        // Bring the DB to a healthy modern schema first so that all of
        // v1..v37's other tables / indexes are present (v19+ reference
        // tables that v17 alone does not create).
        run_migrations(&conn).unwrap();

        // Now reproduce the historical bad state: an `approval_audit`
        // table that is *missing* `second_factor_used`, with
        // `user_version` rolled back to one below SCHEMA_VERSION so that
        // v38 actually re-runs on the next `run_migrations` call. This
        // is exactly what an in-place upgrade from a binary whose
        // `migrate_v17` predated #2131 looks like.
        conn.execute_batch(
            "DROP TABLE approval_audit;
            CREATE TABLE approval_audit (
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
                feedback TEXT
            );",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 37_i32).unwrap();
        // Drop every audit row past v37 — a real DB at user_version=37
        // would never have rows for v38+, and the boot-time ladder
        // invariant (MAX(migrations) <= user_version) refuses to start
        // otherwise.
        conn.execute("DELETE FROM migrations WHERE version > 37", [])
            .unwrap();

        // Sanity: the legacy column is missing before the upgrade runs.
        assert!(
            !column_exists(&conn, "approval_audit", "second_factor_used"),
            "test setup must reproduce the legacy v17 schema (no second_factor_used)"
        );

        // Re-run migrations as a beta.10+ binary would on startup.
        run_migrations(&conn).unwrap();

        // The column must now exist…
        assert!(
            column_exists(&conn, "approval_audit", "second_factor_used"),
            "migrate_v38 must add second_factor_used on the upgrade path"
        );

        // …and an INSERT matching the production statement must succeed.
        // This is the statement that fails on the affected installs and
        // produces the `Failed to write pending audit entry` warning.
        conn.execute(
            "INSERT OR IGNORE INTO approval_audit (
                id, request_id, agent_id, tool_name, description,
                action_summary, risk_level, decision, decided_by,
                decided_at, requested_at, feedback, second_factor_used
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                "audit-1",
                "req-1",
                "agent-1",
                "shell_exec",
                "echo hello",
                "echo hello",
                "low",
                "pending",
                Option::<String>::None,
                "2026-05-11T00:00:00+00:00",
                "2026-05-11T00:00:00+00:00",
                Option::<String>::None,
                false,
            ],
        )
        .unwrap();
    }

    /// Re-running migrations on a healthy DB must be a no-op for v38 —
    /// the `column_exists` guard must short-circuit the ALTER so a
    /// second boot doesn't error with `duplicate column name`.
    #[test]
    fn test_migrate_v38_is_idempotent_on_healthy_schema() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "approval_audit", "second_factor_used"));
        // Second pass must succeed even though the column is already present.
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
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
    fn test_migrate_v47_adds_peer_id_columns() {
        // #6494: entities and relations gain a nullable peer_id for per-user
        // KG isolation. Pre-existing rows must keep working with NULL peer_id.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "entities", "peer_id"));
        assert!(column_exists(&conn, "relations", "peer_id"));

        // A legacy-shaped INSERT that omits peer_id gets the '' shared default.
        conn.execute(
            "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id) VALUES ('leg', '\"Person\"', 'X', '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 'a')",
            [],
        )
        .unwrap();
        let peer: String = conn
            .query_row("SELECT peer_id FROM entities WHERE id = 'leg'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            peer, "",
            "omitted peer_id defaults to the '' shared sentinel"
        );

        // Two users' same-named entities collapse to the same `id` (derived
        // from the name), so both must be insertable — proving the composite
        // PRIMARY KEY (id, peer_id) took effect instead of the old single `id`.
        for peer in ["user-A", "user-B"] {
            conn.execute(
                "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id, peer_id) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
                rusqlite::params!["john", "\"Person\"", "John", "{}", "2026-07-19T00:00:00+00:00", "shared-agent", peer],
            )
            .unwrap();
        }
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities WHERE id = 'john'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            n, 2,
            "same-named entities coexist as one row per peer (#6494)"
        );

        // Shared (empty-peer) entities dedup: inserting the same shared id
        // twice must collapse to one row (NULL would not, hence the '' sentinel).
        conn.execute(
            "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id, peer_id) VALUES ('shared1', '\"Org\"', 'Acme', '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 'a', '')",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id, peer_id) VALUES ('shared1', '\"Org\"', 'Acme', '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 'a', '')",
            [],
        );
        assert!(
            dup.is_err(),
            "a duplicate shared entity must violate the (id, '') PK"
        );
    }

    #[test]
    fn test_migrate_v47_preserves_existing_entity_rows() {
        // The entities rebuild must not lose data: an entity written under the
        // pre-v47 schema must survive with peer_id = NULL (shared).
        let conn = Connection::open_in_memory().unwrap();
        // Migrate only up to the state just before v47 by running the full
        // ladder, then asserting the row seeded post-migration round-trips. We
        // seed after migration (the table shape is stable) and re-run
        // migrate_v47 to prove idempotence does not drop it either.
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id, peer_id) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, '')",
            rusqlite::params!["legacy1", "\"Org\"", "Acme", "{\"k\":1}", "2026-07-19T00:00:00+00:00", "agent-1"],
        )
        .unwrap();

        // A second call to migrate_v47 is a no-op (peer_id already present) and
        // must leave the seeded row untouched.
        migrate_v47(&conn).unwrap();
        let (name, props, peer): (String, String, String) = conn
            .query_row(
                "SELECT name, properties, peer_id FROM entities WHERE id = 'legacy1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "Acme");
        assert_eq!(props, "{\"k\":1}");
        assert_eq!(peer, "", "a pre-v47 row migrates to the '' shared sentinel");
    }

    #[test]
    fn test_migrate_v51_adds_embedding_model_column_nullable_and_indexed() {
        // #7912: `memories.embedding` carried no provenance, so swapping
        // `[memory] embedding_model` between two models of the same
        // dimensionality left every stored vector in place and cosine ran
        // across two unrelated spaces with no error anywhere.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "memories", "embedding_model"));

        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name = 'idx_memories_embedding_model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            index_count, 1,
            "the census and any future re-embedding sweep group by this column over the whole table"
        );

        // A legacy-shaped INSERT that omits embedding_model leaves it NULL,
        // which the recall guard reads as "provenance unknown, still comparable".
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, embedding) \
             VALUES ('legacy-vec', 'agent-1', 'c', 'conversation', 'episodic', 1.0, '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 0, 0, X'00000000')",
            [],
        )
        .unwrap();
        let stamp: Option<String> = conn
            .query_row(
                "SELECT embedding_model FROM memories WHERE id = 'legacy-vec'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            stamp, None,
            "a pre-v51 vector must read back NULL, not a synthetic model name"
        );

        // Re-running is a no-op on both the column and the index, and must not
        // disturb the seeded row.
        migrate_v51(&conn).unwrap();
        migrate_v51(&conn).unwrap();
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name = 'idx_memories_embedding_model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            index_count, 1,
            "the index must exist exactly once after repeated boots"
        );
        let content: String = conn
            .query_row(
                "SELECT content FROM memories WHERE id = 'legacy-vec'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "c", "re-running migrate_v51 must not touch data");
    }

    #[test]
    fn test_migrate_v48_adds_last_decayed_at_column() {
        // #7756: confidence decay needs to know when it last ran for a row.
        // The column is nullable with no default so every pre-migration memory
        // reads back NULL, which the decay pass treats as "charge from
        // accessed_at" — no row jumps at the migration boundary.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "memories", "last_decayed_at"));

        // A legacy-shaped INSERT that omits last_decayed_at leaves it NULL.
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted) \
             VALUES ('legacy-mem', 'agent-1', 'c', 'conversation', 'agent_memory', 1.0, '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 0, 0)",
            [],
        )
        .unwrap();
        let stamp: Option<String> = conn
            .query_row(
                "SELECT last_decayed_at FROM memories WHERE id = 'legacy-mem'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            stamp, None,
            "a pre-v48 memory row must read back NULL, not a synthetic timestamp"
        );

        // A second call is a no-op (column already present) and must not
        // disturb the seeded row.
        migrate_v48(&conn).unwrap();
        let confidence: f64 = conn
            .query_row(
                "SELECT confidence FROM memories WHERE id = 'legacy-mem'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            confidence, 1.0,
            "re-running migrate_v48 must not touch data"
        );
    }

    #[test]
    fn test_migrate_v49_adds_owner_and_billed_agent_columns() {
        // #7714: a run records who asked for it, and a usage row records whose
        // budget line its cost belongs on. Both columns are nullable with no
        // default, so every pre-migration row reads back NULL — an ownerless
        // run stays ownerless and an unbilled usage row still bills to
        // `agent_id`, which is exactly the pre-migration behaviour.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "workflow_runs", "owner_agent_id"));
        assert!(column_exists(&conn, "usage_events", "billed_agent_id"));

        // Legacy-shaped INSERTs that omit the new columns leave them NULL.
        conn.execute(
            "INSERT INTO workflow_runs (id, workflow_id, workflow_name, state, input, step_results, started_at) \
             VALUES ('legacy-run', 'wf-1', 'wf', 'completed', 'in', '[]', '2026-07-19T00:00:00+00:00')",
            [],
        )
        .unwrap();
        let owner: Option<String> = conn
            .query_row(
                "SELECT owner_agent_id FROM workflow_runs WHERE id = 'legacy-run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            owner, None,
            "a pre-v49 workflow run must read back an absent owner, not a synthetic one"
        );

        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
             VALUES ('legacy-usage', 'agent-1', '2026-07-19T00:00:00+00:00', 'm', 'p', 1, 2, 0.5, 0, 10)",
            [],
        )
        .unwrap();
        let billed: Option<String> = conn
            .query_row(
                "SELECT billed_agent_id FROM usage_events WHERE id = 'legacy-usage'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            billed, None,
            "a pre-v49 usage row must read back NULL so it still bills to agent_id"
        );

        // A second call is a no-op (both columns already present) and must not
        // disturb either seeded row.
        migrate_v49(&conn).unwrap();
        let (state, cost): (String, f64) = (
            conn.query_row(
                "SELECT state FROM workflow_runs WHERE id = 'legacy-run'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT cost_usd FROM usage_events WHERE id = 'legacy-usage'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
        );
        assert_eq!(
            state, "completed",
            "re-running migrate_v49 must not touch data"
        );
        assert_eq!(cost, 0.5, "re-running migrate_v49 must not touch data");
    }

    /// #7752: the `ephemeral_runs` table must exist after the ladder, accept a row, refuse an out-of-domain `status`, and survive a re-run untouched.
    #[test]
    fn test_migrate_v53_creates_ephemeral_runs_table() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO ephemeral_runs (
                 id, parent_agent_id, label, worker_name, status,
                 started_at, finished_at, cost_usd
             ) VALUES ('r1', 'parent-1', 'researcher', 'researcher-00ff', 'completed',
                       '2026-01-01T00:00:00Z', '2026-01-01T00:00:05Z', 0.25)",
            [],
        )
        .unwrap();

        // The CHECK constraint is the last line of defence behind the store's own status validation; an unknown value must never round-trip.
        assert!(
            conn.execute(
                "INSERT INTO ephemeral_runs (
                     id, parent_agent_id, label, worker_name, status,
                     started_at, finished_at
                 ) VALUES ('r2', 'parent-1', 'x', 'x-00ff', 'sideways',
                           '2026-01-01T00:00:00Z', '2026-01-01T00:00:05Z')",
                [],
            )
            .is_err(),
            "status must be constrained to the run outcomes the store writes"
        );

        migrate_v53(&conn).unwrap();
        let (n, cost): (i64, f64) = conn
            .query_row(
                "SELECT COUNT(*), IFNULL(SUM(cost_usd), 0.0) FROM ephemeral_runs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(n, 1, "re-running migrate_v53 must not touch data");
        assert_eq!(cost, 0.25, "re-running migrate_v53 must not touch data");
    }

    #[test]
    fn test_migrate_v50_creates_memories_fts_and_keeps_it_in_step() {
        // #7808: the schema had exactly one FTS5 table (`sessions_fts`) and
        // nothing over `memories`, so recall with no embedding provider was a
        // `content LIKE '%…%'` scan and `memory.fts_only` had no memories index
        // to fall back to.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
                [],
                |row| row.get(0),
            )
            .expect("memories_fts must exist after the ladder runs");
        assert!(
            sql.contains("unicode61"),
            "the tokenizer must be declared explicitly, per the #3548 lesson on sessions_fts; got: {sql}"
        );

        // Writes reach the index through triggers, so no write path in the
        // crate has to remember to maintain it.
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted) \
             VALUES ('m1', 'agent-1', 'the release train leaves on Thursday', 'conversation', 'agent_memory', 1.0, '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 0, 0)",
            [],
        )
        .unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
                ["\"release\" OR \"train\""],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "an inserted memory must be searchable immediately");

        // Soft deletes stay indexed on purpose: the query path filters
        // `deleted = 0` against `memories` anyway, and an undelete then needs
        // no re-index.
        conn.execute("UPDATE memories SET deleted = 1 WHERE id = 'm1'", [])
            .unwrap();
        let after_soft_delete: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after_soft_delete, 1);

        // Re-running the migration converges rather than double-inserting —
        // the backfill is a rebuild, not an append.
        migrate_v50(&conn).unwrap();
        let after_rerun: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            after_rerun, 1,
            "re-running migrate_v50 must not duplicate index rows"
        );
    }

    #[test]
    fn test_migrate_v50_backfills_memories_written_before_the_index_existed() {
        // The realistic upgrade: rows already sitting in `memories` when the
        // index is created. Without the backfill an existing store would look
        // empty to every FTS-backed search until each row happened to be
        // rewritten.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Reproduce the pre-v49 state: rows present, index absent.
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted) \
             VALUES ('old', 'agent-1', 'the postgres primary lives in Frankfurt', 'conversation', 'agent_memory', 1.0, '{}', '2026-07-19T00:00:00+00:00', '2026-07-19T00:00:00+00:00', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "DROP TRIGGER memories_fts_ai;
             DROP TRIGGER memories_fts_au;
             DROP TRIGGER memories_fts_ad;
             DROP TABLE memories_fts;",
        )
        .unwrap();

        migrate_v50(&conn).unwrap();

        let (id, _): (String, String) = conn
            .query_row(
                "SELECT memory_id, content FROM memories_fts WHERE memories_fts MATCH ?1",
                ["\"postgres\""],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("a row written before the index existed must be backfilled");
        assert_eq!(id, "old");
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
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn test_migrate_v40_ladderizes_agents_columns_and_session_generation_5138() {
        // #5138: the three `agents` schema-evolution columns were
        // previously bolted on by per-save ALTERs that bypassed the
        // ladder. v40 must declare them in-ladder and add
        // `sessions.messages_generation`, all reflected in the audit
        // trail / user_version.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "agents", "session_id"));
        assert!(column_exists(&conn, "agents", "identity"));
        assert!(column_exists(&conn, "agents", "source_toml_path"));
        assert!(column_exists(&conn, "sessions", "messages_generation"));

        // The migration must be recorded in the audit trail so
        // `user_version` and `SELECT version FROM migrations` agree.
        let v40_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE version = 40",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(v40_rows, 1, "v40 must record its audit row");
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn test_migrate_v40_is_idempotent_5138() {
        // Re-running on an already-v40 schema (and on a DB that already
        // carries the columns from the legacy per-save ALTER) must no-op
        // cleanly via the `column_exists` guard.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "agents", "session_id"));
        assert!(column_exists(&conn, "sessions", "messages_generation"));
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn).unwrap(), 9);

        // Resuming migrations from this state must succeed without
        // "duplicate column name: agent_id".
        run_migrations(&conn).expect("v10 retry on partial-apply DB must not error");
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);

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
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(column_exists(&conn, "entities", "agent_id"));
        assert!(column_exists(&conn, "relations", "agent_id"));
    }

    /// Regression for #4898: v39 must add `model_override TEXT` to `sessions`,
    /// preserve NULL for pre-existing rows, and persist the value on write/read.
    #[test]
    fn migrate_v39_adds_model_override_column_nullable_and_round_trips() {
        // Start from a v38-equivalent schema (full run_migrations populates
        // the column through v39, but we want to verify the column shape).
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Column must exist after migration.
        assert!(
            column_exists(&conn, "sessions", "model_override"),
            "sessions.model_override column missing after migrate_v39"
        );

        // Insert a minimal sessions row — model_override should default to NULL
        // (backwards compatible). `created_at` and `updated_at` are NOT NULL.
        let sid = uuid::Uuid::new_v4().to_string();
        let aid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, messages, created_at, updated_at) \
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))",
            rusqlite::params![sid, aid, b"[]" as &[u8]],
        )
        .unwrap();

        let stored_override: Option<String> = conn
            .query_row(
                "SELECT model_override FROM sessions WHERE id = ?1",
                [&sid],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            stored_override.is_none(),
            "model_override must default to NULL for pre-migration rows: got {stored_override:?}"
        );

        // Write a non-NULL override and read it back.
        conn.execute(
            "UPDATE sessions SET model_override = ?1 WHERE id = ?2",
            rusqlite::params!["groq/llama-3.3-70b", sid],
        )
        .unwrap();

        let stored_override: Option<String> = conn
            .query_row(
                "SELECT model_override FROM sessions WHERE id = ?1",
                [&sid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stored_override.as_deref(),
            Some("groq/llama-3.3-70b"),
            "model_override round-trip failed: {stored_override:?}"
        );

        // Clear back to NULL.
        conn.execute(
            "UPDATE sessions SET model_override = NULL WHERE id = ?1",
            [&sid],
        )
        .unwrap();
        let cleared: Option<String> = conn
            .query_row(
                "SELECT model_override FROM sessions WHERE id = ?1",
                [&sid],
                |row| row.get(0),
            )
            .unwrap();
        assert!(cleared.is_none(), "clearing model_override to NULL failed");
    }

    /// Audit: sessions-missing-index. After v41 both
    /// `idx_sessions_agent_updated` (composite on sessions) and
    /// `idx_audit_agent_timestamp` (composite on audit_entries) must
    /// exist, and the planner must pick the new sessions index for
    /// the `agent_id + updated_at` hot path that
    /// `count_agent_sessions_touched_since` runs.
    #[test]
    fn migrate_v41_creates_composite_indexes_used_by_hot_paths() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let sessions_idx: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master \
                 WHERE type='index' AND name='idx_sessions_agent_updated'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(
            sessions_idx.as_deref(),
            Some("idx_sessions_agent_updated"),
            "v41 must create idx_sessions_agent_updated on sessions(agent_id, updated_at)"
        );

        let audit_idx: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master \
                 WHERE type='index' AND name='idx_audit_agent_timestamp'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(
            audit_idx.as_deref(),
            Some("idx_audit_agent_timestamp"),
            "v41 must create idx_audit_agent_timestamp on audit_entries(agent_id, timestamp)"
        );

        // Verify the planner actually picks the new index for the
        // canonical hot-path query. `EXPLAIN QUERY PLAN` returns
        // a row whose `detail` column (index 3) names the chosen
        // strategy. The audit doc's regression is "degrades to
        // full-table scan" — we assert against that by requiring
        // the new index name appears in the plan.
        let plan: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN \
                 SELECT COUNT(*) FROM sessions WHERE agent_id = ?1 AND updated_at > ?2",
            )
            .unwrap()
            .query_map(["a", "2000-01-01"], |row| row.get::<_, String>(3))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        let plan_text = plan.join("\n");
        assert!(
            plan_text.contains("idx_sessions_agent_updated"),
            "count_agent_sessions_touched_since must use the new composite index, \
             planner picked: {plan_text}"
        );
    }

    #[test]
    fn migrate_v41_is_idempotent() {
        // Re-running migrations must not fail or change anything. The
        // ladder runner exits at user_version = SCHEMA_VERSION and the
        // index DDL itself uses IF NOT EXISTS as a second safety net.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_sessions_agent_updated'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "v41 sessions index must exist exactly once after repeated boots"
        );
    }

    // ---------------------------------------------------------------------
    // v54 (#7930): agent lineage persistence
    // ---------------------------------------------------------------------

    #[test]
    fn v54_adds_parent_columns_and_index() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(
            column_exists(&conn, "agents", "parent_id"),
            "v54 must add agents.parent_id — without it AgentEntry.parent has nowhere to live"
        );
        assert!(
            column_exists(&conn, "agents", "parent_recorded"),
            "v54 must add agents.parent_recorded so a pre-v54 row is distinguishable from a root agent"
        );

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_agents_parent_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "idx_agents_parent_id is what makes deriving `children` a lookup instead of a scan"
        );
    }

    #[test]
    fn v54_is_idempotent_across_repeated_boots() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_agents_parent_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "v54 index must exist exactly once after repeated boots"
        );
    }

    #[test]
    fn v54_records_its_audit_row_under_its_own_version() {
        // `migrate_v50` recorded itself as version 49 (#7924/#7925), which desynchronises the audit trail from `user_version`.
        // Guard against that copy-paste for v54 specifically.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let description: String = conn
            .query_row(
                "SELECT description FROM migrations WHERE version = 54",
                [],
                |row| row.get(0),
            )
            .expect("v54 must have recorded an audit row under version 54");
        assert!(
            description.contains("#7930"),
            "the v54 audit row must be v54's own, not a backfill placeholder or another migration's: {description}"
        );
    }

    #[test]
    fn v54_marks_pre_existing_agent_rows_as_lineage_unknown() {
        // The regression this guards: `ALTER TABLE ... ADD COLUMN parent_id` backfills NULL, and NULL is also what a genuine root agent stores.
        // If `parent_recorded` did not default to 0, every agent that predates v54 would start positively claiming to be a root agent — a more confident wrong answer than the `null` the bug already produced.
        //
        // Simulates a real pre-v54 database: build the v40-era `agents` table, insert a row, stamp `user_version = 50`, then let the ladder run.
        // Steps 51-54 all fire from 50, so the fixture also needs the tables 51 and 52 alter — `memories` and `group_roster` — even though this test asserts nothing about them.
        // Their absence is not a v54 bug; a real database at user_version 50 has both.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                manifest BLOB NOT NULL,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                session_id TEXT DEFAULT '',
                identity TEXT DEFAULT '{}',
                source_toml_path TEXT DEFAULT NULL
            );
            CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );
            CREATE TABLE group_roster (
                chat_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                PRIMARY KEY (chat_id, user_id)
            );
            CREATE TABLE migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL,
                description TEXT NOT NULL
            );
            INSERT INTO agents (id, name, manifest, state, created_at, updated_at)
            VALUES ('legacy-agent', 'legacy', x'80', '\"Running\"', '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z');
            ",
        )
        .unwrap();
        set_schema_version(&conn, 50).unwrap();

        run_migrations(&conn).unwrap();

        let (parent_id, parent_recorded): (Option<String>, i64) = conn
            .query_row(
                "SELECT parent_id, parent_recorded FROM agents WHERE id = 'legacy-agent'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(
            parent_id, None,
            "a pre-v54 row has no recoverable parent id"
        );
        assert_eq!(
            parent_recorded, 0,
            "a pre-v54 row must read as UNKNOWN lineage, not as a root agent"
        );
    }
}
