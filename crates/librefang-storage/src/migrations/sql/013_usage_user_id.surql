-- Migration v13: Add user_id and channel columns to usage_events (RBAC M5 per-user spend rollup).
--
-- Mirrors SQLite migration v23. Pre-existing rows will have NULL values —
-- the per-user spend queries skip NULL rows by design (`WHERE user_id = $user_id`
-- never matches NULL in SurrealDB).
--
-- The composite index on (user_id, timestamp) makes the per-user hourly /
-- daily / monthly rollup queries efficient.

DEFINE FIELD IF NOT EXISTS user_id ON usage_events TYPE option<string>;
DEFINE FIELD IF NOT EXISTS channel ON usage_events TYPE option<string>;

DEFINE INDEX IF NOT EXISTS usage_user_time_idx
    ON usage_events COLUMNS user_id, timestamp;
