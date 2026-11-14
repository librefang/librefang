-- Migration v40: mcp_server_configs table for database-backed MCP server config.
-- Mirrors upstream SQLite v43 (migrate_v43, upstream #6021).
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS mcp_server_configs (
--       name       TEXT PRIMARY KEY,
--       entry_json TEXT NOT NULL,
--       created_at TEXT NOT NULL DEFAULT (datetime('now')),
--       updated_at TEXT NOT NULL DEFAULT (datetime('now'))
--   );
--
-- `entry_json` stays an opaque `string` rather than a SurrealDB `object`: it is
-- a serialized `McpServerEntry` whose shape is owned by librefang-types, and
-- storing it as text keeps one serializer responsible for the contents instead
-- of splitting it across the type and a schema declaration here.

DEFINE TABLE IF NOT EXISTS mcp_server_configs SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS name       ON mcp_server_configs TYPE string;
DEFINE FIELD IF NOT EXISTS entry_json ON mcp_server_configs TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON mcp_server_configs TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON mcp_server_configs TYPE string;

DEFINE INDEX IF NOT EXISTS idx_mcp_server_configs_name
    ON mcp_server_configs FIELDS name UNIQUE;
