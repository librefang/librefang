-- Phase 12 / migration v8: LLM usage events for metering and quota enforcement.
--
-- Mirrors the rusqlite `usage_events` table including the `latency_ms` and
-- `provider` columns added in migrations v14 and v19.

DEFINE TABLE IF NOT EXISTS usage_events SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id      ON usage_events TYPE string;
DEFINE FIELD IF NOT EXISTS timestamp     ON usage_events TYPE string;
DEFINE FIELD IF NOT EXISTS provider      ON usage_events TYPE string;
DEFINE FIELD IF NOT EXISTS model         ON usage_events TYPE string;
DEFINE FIELD IF NOT EXISTS input_tokens  ON usage_events TYPE int;
DEFINE FIELD IF NOT EXISTS output_tokens ON usage_events TYPE int;
DEFINE FIELD IF NOT EXISTS cost_usd      ON usage_events TYPE float;
DEFINE FIELD IF NOT EXISTS tool_calls    ON usage_events TYPE int;
DEFINE FIELD IF NOT EXISTS latency_ms    ON usage_events TYPE int;

DEFINE INDEX IF NOT EXISTS usage_agent_time_idx
    ON usage_events COLUMNS agent_id, timestamp;
DEFINE INDEX IF NOT EXISTS usage_time_idx
    ON usage_events COLUMNS timestamp;
DEFINE INDEX IF NOT EXISTS usage_provider_idx
    ON usage_events COLUMNS provider;
