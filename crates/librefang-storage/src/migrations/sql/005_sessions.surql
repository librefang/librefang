-- Phase 8 / migration v5: per-agent session history and canonical sessions.
--
-- `sessions` mirrors the rusqlite `sessions` table layout.  `messages` and
-- other arbitrary JSON payloads are stored FLEXIBLE so nested structures
-- survive the SurrealDB SCHEMAFULL round-trip without requiring exhaustive
-- field declarations.

DEFINE TABLE IF NOT EXISTS sessions SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id              ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS messages              ON sessions TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS context_window_tokens ON sessions TYPE int;
DEFINE FIELD IF NOT EXISTS created_at            ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at            ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS label                 ON sessions TYPE option<string>;

DEFINE INDEX IF NOT EXISTS sessions_agent_idx ON sessions COLUMNS agent_id;

-- `canonical_sessions` stores the cross-channel persistent memory per agent.
-- One row per agent (agent_id is the unique key, used as the record id).

DEFINE TABLE IF NOT EXISTS canonical_sessions SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id          ON canonical_sessions TYPE string;
DEFINE FIELD IF NOT EXISTS messages          ON canonical_sessions TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS compaction_cursor ON canonical_sessions TYPE int;
DEFINE FIELD IF NOT EXISTS compacted_summary ON canonical_sessions TYPE option<string>;
DEFINE FIELD IF NOT EXISTS updated_at        ON canonical_sessions TYPE string;

DEFINE INDEX IF NOT EXISTS canonical_sessions_agent_idx
    ON canonical_sessions COLUMNS agent_id UNIQUE;
