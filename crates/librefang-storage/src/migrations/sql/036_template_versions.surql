-- Migration v36: template_versions table for agent-type (template) version history.
-- Mirrors upstream SQLite v55 (migrate_v55).
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS template_versions (
--       id            INTEGER PRIMARY KEY AUTOINCREMENT,
--       template_name TEXT NOT NULL,
--       timestamp     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
--       manifest_toml TEXT NOT NULL,
--       change_source TEXT NOT NULL DEFAULT 'unknown'
--   );
--   CREATE INDEX IF NOT EXISTS idx_template_versions_name
--       ON template_versions(template_name, timestamp DESC);
--
-- The SQLite surrogate `id INTEGER PRIMARY KEY AUTOINCREMENT` has no mirrored
-- field: SurrealDB record ids are the primary key. Timestamps stay `string`
-- (RFC3339 text) to match 027_workflow_runs and 035_manifest_versions rather
-- than diverging to `datetime` for one table.

DEFINE TABLE IF NOT EXISTS template_versions SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS template_name ON template_versions TYPE string;
DEFINE FIELD IF NOT EXISTS timestamp     ON template_versions TYPE string;
DEFINE FIELD IF NOT EXISTS manifest_toml ON template_versions TYPE string;
DEFINE FIELD IF NOT EXISTS change_source ON template_versions TYPE string VALUE $value OR 'unknown';

DEFINE INDEX IF NOT EXISTS idx_template_versions_name
    ON template_versions FIELDS template_name, timestamp;
