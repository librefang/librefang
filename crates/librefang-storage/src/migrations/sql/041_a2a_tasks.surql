-- Migration v41: a2a_tasks table for A2A protocol task persistence.
--
-- Mirrors the schema `A2aTaskStore::with_persistence` creates by hand in
-- crates/librefang-runtime/src/a2a.rs (table `a2a_tasks_v2`), which is the
-- second SQLite file the daemon opens (`<data_dir>/a2a_tasks.db`) alongside
-- `librefang.db`. It is not part of the librefang-memory migration ladder, so
-- it has no upstream `migrate_vNN` counterpart to cite.
--
-- Source DDL:
--   CREATE TABLE IF NOT EXISTS a2a_tasks_v2 (
--       id                  TEXT PRIMARY KEY,
--       status              TEXT NOT NULL,
--       session_id          TEXT,
--       messages_json       TEXT NOT NULL,
--       artifacts_json      TEXT NOT NULL,
--       agent_id            TEXT,
--       caller_a2a_agent_id TEXT,
--       created_at          INTEGER NOT NULL,
--       updated_at          INTEGER NOT NULL
--   );
--
-- The table is named `a2a_tasks` here, not `a2a_tasks_v2`: the `_v2` suffix is
-- a SQLite in-place migration artefact (the original shape was replaced rather
-- than altered), and carrying it into a fresh schema would preserve a rename
-- that never happened on this backend.
--
-- `messages_json` / `artifacts_json` stay opaque `string`: they serialize
-- `Vec<A2aMessage>` and `Vec<A2aArtifact>`, whose shapes belong to a2a.rs.
-- Timestamps are `int` (unix seconds), matching the source columns.

DEFINE TABLE IF NOT EXISTS a2a_tasks SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS id                  ON a2a_tasks TYPE string;
DEFINE FIELD IF NOT EXISTS status              ON a2a_tasks TYPE string;
DEFINE FIELD IF NOT EXISTS session_id          ON a2a_tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS messages_json       ON a2a_tasks TYPE string VALUE $value OR '[]';
DEFINE FIELD IF NOT EXISTS artifacts_json      ON a2a_tasks TYPE string VALUE $value OR '[]';
DEFINE FIELD IF NOT EXISTS agent_id            ON a2a_tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS caller_a2a_agent_id ON a2a_tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at          ON a2a_tasks TYPE int;
DEFINE FIELD IF NOT EXISTS updated_at          ON a2a_tasks TYPE int;

DEFINE INDEX IF NOT EXISTS idx_a2a_tasks_id         ON a2a_tasks FIELDS id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_a2a_tasks_updated_at ON a2a_tasks FIELDS updated_at;
DEFINE INDEX IF NOT EXISTS idx_a2a_tasks_agent_id   ON a2a_tasks FIELDS agent_id;
