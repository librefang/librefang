-- Phase 9 / migration v6: per-agent key-value store.
--
-- Mirrors the rusqlite `kv_store` table.  `value` is FLEXIBLE to allow
-- arbitrary JSON (the kernel stores goal configs and other structured blobs).
-- The compound (agent_id, key) index is UNIQUE — matching the SQLite PRIMARY
-- KEY (agent_id, key) semantics.

DEFINE TABLE IF NOT EXISTS kv_store SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id   ON kv_store TYPE string;
DEFINE FIELD IF NOT EXISTS key        ON kv_store TYPE string;
DEFINE FIELD IF NOT EXISTS value      ON kv_store TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS version    ON kv_store TYPE int;
DEFINE FIELD IF NOT EXISTS updated_at ON kv_store TYPE string;

DEFINE INDEX IF NOT EXISTS kv_store_agent_key_idx
    ON kv_store COLUMNS agent_id, key UNIQUE;
