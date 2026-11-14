-- Migration v32: record which session owns a canonical compacted summary.
-- Mirrors upstream SQLite v46 (#6225): ALTER TABLE canonical_sessions ADD COLUMN compacted_summary_session_id TEXT.
--
-- `canonical_sessions.compacted_summary` is agent-scoped (one row per agent)
-- and outlives any single session. Without an owner pointer the compaction
-- banner was surfaced on every session — including a freshly created one whose
-- own history was never compacted. This field records the session whose history
-- produced `compacted_summary`, so the API only shows the banner on the
-- compacted session. NULL (legacy rows / no owner) suppresses the banner.
--
-- canonical_sessions is SCHEMAFULL, so the field must be declared here or the
-- value would be silently dropped on write (and lost by the sqlite→surreal
-- migration copy in migrate/sqlite_to_surreal.rs).

DEFINE FIELD IF NOT EXISTS compacted_summary_session_id ON canonical_sessions TYPE option<string>;
