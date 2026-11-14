-- Migration v20: Add session_id column to usage_events for per-session cost rollup.
--
-- Mirrors SQLite migration v30. Pre-existing rows will have NULL values —
-- per-session spend queries skip NULL rows by design (`WHERE session_id = $session_id`
-- never matches NULL in SurrealDB).
--
-- The index on session_id makes per-session cost rollup queries efficient.

DEFINE FIELD IF NOT EXISTS session_id ON usage_events TYPE option<string>;

DEFINE INDEX IF NOT EXISTS idx_usage_session
    ON usage_events COLUMNS session_id;
