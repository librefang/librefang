-- Migration v51: add reasoning_effort field to sessions for the per-session reasoning-effort pin.
-- Mirrors upstream SQLite v65 (#8591): ALTER TABLE sessions ADD COLUMN reasoning_effort TEXT DEFAULT NULL.
--
-- NONE means "inherit" (the agent manifest / kernel default applies), matching upstream's NULL.
-- The value is a reasoning-effort level string as written by `set_session_reasoning_effort`.
-- Upstream writes the column only through that setter, never through a routine history save, so a save must not clear the pin.
-- `sessions` is SCHEMAFULL, so without this field the pin is dropped on write and on the SQLite -> SurrealDB import.

DEFINE FIELD IF NOT EXISTS reasoning_effort ON sessions TYPE option<string>;
