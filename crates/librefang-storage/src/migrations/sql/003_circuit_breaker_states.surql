-- Phase 6 / migration v3: circuit-breaker state per `<plugin>::<hook>` key.

DEFINE TABLE IF NOT EXISTS circuit_breaker_states SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS key       ON circuit_breaker_states TYPE string;
DEFINE FIELD IF NOT EXISTS failures  ON circuit_breaker_states TYPE int;
-- ISO-8601 string when the breaker is open, NULL when closed.
DEFINE FIELD IF NOT EXISTS opened_at ON circuit_breaker_states TYPE option<string>;

DEFINE INDEX IF NOT EXISTS circuit_breaker_states_key_idx
    ON circuit_breaker_states COLUMNS key UNIQUE;
