-- Migration v35: manifest_versions table for agent manifest history.
-- Mirrors upstream SQLite v58 (migrate_v58) and its forward-compat re-run v60.
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS manifest_versions (
--       id            INTEGER PRIMARY KEY AUTOINCREMENT,
--       agent_id      TEXT NOT NULL,
--       agent_name    TEXT NOT NULL DEFAULT '',
--       timestamp     TEXT NOT NULL DEFAULT (datetime('now')),
--       manifest_toml TEXT NOT NULL,
--       change_source TEXT NOT NULL DEFAULT 'unknown',
--       created_at    TEXT NOT NULL DEFAULT (datetime('now'))
--   );
--   CREATE INDEX IF NOT EXISTS idx_manifest_versions_agent_id
--       ON manifest_versions(agent_id, timestamp DESC);
--
-- The SQLite `id INTEGER PRIMARY KEY AUTOINCREMENT` has no field here: SurrealDB
-- record ids are the primary key, so a mirrored surrogate column would be dead
-- weight. Timestamps stay `string` to match the rest of this migration set
-- (027_workflow_runs, 016_pending_approvals), which store RFC3339 text rather
-- than `datetime` — a type change would diverge from how the other tables read.
--
-- NOTE (not addressed by this migration): upstream registers manifest_versions
-- in the agent-delete purge cascade (agent_tables.rs, AgentTableGroup::Structured
-- keyed on agent_id) because these rows hold whole `agent.toml` bodies — system
-- prompts, model choices, capability lists. There is no SurrealDB-side cascade
-- to mirror it into; `AgentTableGroup` exists only in librefang-memory's SQLite
-- path. Deleting an agent therefore does not purge its manifest history on the
-- surreal backend. Tracked for the SurrealDB parity work, not fixable here.

DEFINE TABLE IF NOT EXISTS manifest_versions SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id      ON manifest_versions TYPE string;
DEFINE FIELD IF NOT EXISTS agent_name    ON manifest_versions TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS timestamp     ON manifest_versions TYPE string;
DEFINE FIELD IF NOT EXISTS manifest_toml ON manifest_versions TYPE string;
DEFINE FIELD IF NOT EXISTS change_source ON manifest_versions TYPE string VALUE $value OR 'unknown';
DEFINE FIELD IF NOT EXISTS created_at    ON manifest_versions TYPE string;

DEFINE INDEX IF NOT EXISTS idx_manifest_versions_agent_id
    ON manifest_versions FIELDS agent_id, timestamp;
