-- Migration v43: add parent_session_id to sessions so a sub-agent run records which session spawned it.
-- Mirrors upstream SQLite v61 (migrate_v61, upstream #7752):
--   ALTER TABLE sessions ADD COLUMN parent_session_id TEXT DEFAULT NULL;
--   CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;
--
-- NONE on every ordinary session, which is almost all of them.
-- Stored as the parent's session UUID string, the same shape `agent_id` uses, so a row round-trips through either backend.
--
-- SurrealDB has no partial index, so the `WHERE parent_session_id IS NOT NULL` predicate is dropped.
-- A plain index over an optional field serves the same `children_of` lookup; rows without a parent only add NONE entries.
--
-- Upstream's parent-delete cascade is enforced in librefang-memory's SQLite session code, not in the schema, so there is nothing to mirror here.

DEFINE FIELD IF NOT EXISTS parent_session_id ON sessions TYPE option<string>;

DEFINE INDEX IF NOT EXISTS idx_sessions_parent ON sessions FIELDS parent_session_id;
