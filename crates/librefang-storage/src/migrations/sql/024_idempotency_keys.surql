-- Migration v24: idempotency_keys table for Idempotency-Key replay cache (#3637).
--
-- Mirrors SQLite migration v34. Stores the request fingerprint and cached
-- response so that a second request carrying the same Idempotency-Key header
-- is returned the same response without re-executing the handler.
--
-- Schema matches the rusqlite layout:
--   key             — the client-supplied idempotency key (primary key)
--   body_hash       — SHA-256 of the original request body (detects body mismatch)
--   response_status — HTTP status code of the original response
--   response_body   — serialised response payload (bytes stored as SurrealDB bytes)
--   created_at      — Unix timestamp (seconds) when the key was first seen
--   expires_at      — Unix timestamp (seconds) after which the key may be evicted
--
-- The index on expires_at enables efficient sweeps by the background prune job
-- that periodically removes expired entries.

DEFINE TABLE IF NOT EXISTS idempotency_keys SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS key             ON idempotency_keys TYPE string;
DEFINE FIELD IF NOT EXISTS body_hash       ON idempotency_keys TYPE string;
DEFINE FIELD IF NOT EXISTS response_status ON idempotency_keys TYPE int;
DEFINE FIELD IF NOT EXISTS response_body   ON idempotency_keys TYPE bytes;
DEFINE FIELD IF NOT EXISTS created_at      ON idempotency_keys TYPE int;
DEFINE FIELD IF NOT EXISTS expires_at      ON idempotency_keys TYPE int;

DEFINE INDEX IF NOT EXISTS idempotency_keys_pk
    ON idempotency_keys COLUMNS key UNIQUE;

DEFINE INDEX IF NOT EXISTS idx_idempotency_keys_expires_at
    ON idempotency_keys COLUMNS expires_at;
