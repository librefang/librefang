-- Migration v37: goal_runs table for per-agent autonomous goal-run state.
-- Mirrors upstream SQLite v42 (migrate_v42).
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS goal_runs (
--       goal_id        TEXT PRIMARY KEY,
--       agent_id       TEXT NOT NULL,
--       phase          TEXT NOT NULL CHECK (phase IN
--                        ('running','finished','max_iterations_reached','rate_limited','stopped')),
--       iteration      INTEGER NOT NULL DEFAULT 0,
--       max_iterations INTEGER NOT NULL DEFAULT 0,
--       last_progress  INTEGER NOT NULL DEFAULT 0,
--       last_error     TEXT,
--       started_at     TEXT NOT NULL,
--       updated_at     TEXT NOT NULL,
--       created_at     TEXT NOT NULL DEFAULT (datetime('now'))
--   );
--   CREATE INDEX idx_goal_runs_phase      ON goal_runs(phase);
--   CREATE INDEX idx_goal_runs_started_at ON goal_runs(started_at DESC);
--
-- `goal_id` is the SQLite primary key; here it stays an indexed field and the
-- record id carries identity, matching how 027_workflow_runs treats its `id`.
-- The SQLite CHECK constraint becomes an ASSERT, as 027_workflow_runs does for
-- its own `state` column.

DEFINE TABLE IF NOT EXISTS goal_runs SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS goal_id        ON goal_runs TYPE string;
DEFINE FIELD IF NOT EXISTS agent_id       ON goal_runs TYPE string;
DEFINE FIELD IF NOT EXISTS phase          ON goal_runs TYPE string
    ASSERT $value IN ['running', 'finished', 'max_iterations_reached', 'rate_limited', 'stopped'];
DEFINE FIELD IF NOT EXISTS iteration      ON goal_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS max_iterations ON goal_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS last_progress  ON goal_runs TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS last_error     ON goal_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS started_at     ON goal_runs TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at     ON goal_runs TYPE string;
DEFINE FIELD IF NOT EXISTS created_at     ON goal_runs TYPE string;

DEFINE INDEX IF NOT EXISTS idx_goal_runs_goal_id    ON goal_runs FIELDS goal_id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_goal_runs_agent_id   ON goal_runs FIELDS agent_id;
DEFINE INDEX IF NOT EXISTS idx_goal_runs_phase      ON goal_runs FIELDS phase;
DEFINE INDEX IF NOT EXISTS idx_goal_runs_started_at ON goal_runs FIELDS started_at;
