-- Migration v42: ephemeral_runs table so a disposable sub-agent run leaves a
-- record under its parent.
-- Mirrors upstream SQLite v53 (migrate_v53, upstream #7752).
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS ephemeral_runs (
--       id              TEXT PRIMARY KEY,
--       parent_agent_id TEXT NOT NULL,
--       label           TEXT NOT NULL,
--       worker_name     TEXT NOT NULL,
--       agent_type      TEXT,
--       task            TEXT NOT NULL DEFAULT '',
--       response        TEXT NOT NULL DEFAULT '',
--       status          TEXT NOT NULL CHECK (status IN ('completed','failed')),
--       error           TEXT,
--       provider        TEXT NOT NULL DEFAULT '',
--       model           TEXT NOT NULL DEFAULT '',
--       iterations      INTEGER NOT NULL DEFAULT 0,
--       tool_calls      INTEGER NOT NULL DEFAULT 0,
--       input_tokens    INTEGER NOT NULL DEFAULT 0,
--       output_tokens   INTEGER NOT NULL DEFAULT 0,
--       cost_usd        REAL NOT NULL DEFAULT 0.0,
--       latency_ms      INTEGER NOT NULL DEFAULT 0,
--       started_at      TEXT NOT NULL,
--       finished_at     TEXT NOT NULL,
--       created_at      TEXT NOT NULL DEFAULT (datetime('now'))
--   );
--   CREATE INDEX IF NOT EXISTS idx_ephemeral_runs_parent
--       ON ephemeral_runs(parent_agent_id, finished_at DESC);
--
-- `cost_usd` is the one REAL column in this set and becomes `float`; rounding it
-- to `int` would silently zero every sub-cent run. The status CHECK becomes an
-- ASSERT, as 027_workflow_runs does for its own `state`.
--
-- Upstream lists this table in the agent-delete purge cascade (agent_tables.rs)
-- keyed on `parent_agent_id`, not `agent_id`: a run is purged with the parent
-- that spawned it. There is no SurrealDB-side cascade to mirror that into yet —
-- `AgentTableGroup` exists only in librefang-memory's SQLite path — so deleting
-- an agent does not currently purge its ephemeral-run history on this backend.

DEFINE TABLE IF NOT EXISTS ephemeral_runs SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS id              ON ephemeral_runs TYPE string;
DEFINE FIELD IF NOT EXISTS parent_agent_id ON ephemeral_runs TYPE string;
DEFINE FIELD IF NOT EXISTS label           ON ephemeral_runs TYPE string;
DEFINE FIELD IF NOT EXISTS worker_name     ON ephemeral_runs TYPE string;
DEFINE FIELD IF NOT EXISTS agent_type      ON ephemeral_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS task            ON ephemeral_runs TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS response        ON ephemeral_runs TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS status          ON ephemeral_runs TYPE string
    ASSERT $value IN ['completed', 'failed'];
DEFINE FIELD IF NOT EXISTS error           ON ephemeral_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS provider        ON ephemeral_runs TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS model           ON ephemeral_runs TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS iterations      ON ephemeral_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS tool_calls      ON ephemeral_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS input_tokens    ON ephemeral_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS output_tokens   ON ephemeral_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS cost_usd        ON ephemeral_runs TYPE float VALUE $value OR 0.0;
DEFINE FIELD IF NOT EXISTS latency_ms      ON ephemeral_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS started_at      ON ephemeral_runs TYPE string;
DEFINE FIELD IF NOT EXISTS finished_at     ON ephemeral_runs TYPE string;
DEFINE FIELD IF NOT EXISTS created_at      ON ephemeral_runs TYPE string;

DEFINE INDEX IF NOT EXISTS idx_ephemeral_runs_id     ON ephemeral_runs FIELDS id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_ephemeral_runs_parent ON ephemeral_runs FIELDS parent_agent_id, finished_at;
