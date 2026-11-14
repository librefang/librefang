-- Migration v50: SurrealDB parity for upstream SQLite v62-v64.
-- Mirrors these SQLite steps from crates/librefang-memory/src/migration.rs:
--   v62: ALTER TABLE task_queue ADD COLUMN timeout_secs INTEGER DEFAULT NULL;
--   v63 (#8556): ALTER TABLE ephemeral_runs ADD COLUMN truncated INTEGER NOT NULL DEFAULT 0;
--   v64: CREATE INDEX idx_sessions_created ON sessions(created_at);
--        CREATE INDEX idx_sessions_agent_created ON sessions(agent_id, created_at);
--        CREATE INDEX idx_memories_agent_created ON memories(deleted, agent_id, created_at);
--        CREATE INDEX idx_memories_created ON memories(deleted, created_at);
--        CREATE TABLE sessions_fts_map (...) plus its AFTER DELETE trigger.
--
-- `task_queue.timeout_secs` is a per-task claim TTL override: NONE inherits `[task_board] claim_ttl_secs`, 0 means never reclaim.
-- The tables are SCHEMAFULL, so without the field a per-task TTL is dropped on write.
--
-- `ephemeral_runs.truncated` marks a worker that hit its iteration cap and still delivered a partial answer, so it reads as `completed` but is distinguishable from a full run.
-- SQLite stores it as INTEGER 0/1; SurrealDB stores a bool that defaults to false for rows written before this migration.
-- SurrealDB 4 refuses a new non-optional field on a populated SCHEMAFULL table in a compatibility-level-4 database, so existing rows are migrated to false, the value SQLite's DEFAULT 0 backfills.
--
-- Not mirrored, on purpose:
--   * `sessions_fts_map` and its trigger exist only to address SQLite FTS5 rows by rowid; SurrealDB has no FTS5 table to map.
--   * The `memories` indexes target the SQLite `memories` table.
--     This schema has no SurrealDB home for semantic memory beyond the retention fields of 019, so there is nothing for them to index.

DEFINE FIELD IF NOT EXISTS timeout_secs ON task_queue TYPE option<int>;

DEFINE FIELD IF NOT EXISTS truncated ON ephemeral_runs TYPE bool VALUE $value OR false
    MIGRATE EXISTING ROWS AS ephemeral_runs_truncated_v50 USING $value OR false FALLBACK false;

DEFINE INDEX IF NOT EXISTS idx_sessions_created       ON sessions FIELDS created_at;
DEFINE INDEX IF NOT EXISTS idx_sessions_agent_created ON sessions FIELDS agent_id, created_at;
