-- Migration v22: Add message_count field to sessions for fast list rendering (#3607).
--
-- Mirrors SQLite migration v32. Adds a denormalised counter that tracks the
-- number of messages in a session without requiring the full messages blob to
-- be deserialised on every list_sessions / list_agent_sessions call.
--
-- The SurrealDB SessionBackend (surreal_session.rs) already computes and
-- writes `message_count = session.messages.len()` on every save_session call.
-- Without this DEFINE FIELD the table is SCHEMAFULL and rejects the field on
-- write, causing save_session to error and breaking the agent message loop.
--
-- Pre-existing rows will have message_count = 0 (default) until the next
-- save_session overwrites them — callers treat 0 as "unknown" and fall back
-- to blob deserialisation for those rows.
--
-- VALUE $value OR 0 ensures the field is always an integer (never null).

DEFINE FIELD IF NOT EXISTS message_count ON sessions TYPE int VALUE $value OR 0;
