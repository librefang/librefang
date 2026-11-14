-- Migration v39: channel_instance_defaults + conversation_bindings for
-- deterministic inbound channel dispatch.
-- Mirrors upstream SQLite v45 (migrate_v45, upstream #5671), which creates both
-- tables in one step — they are kept together here for the same reason: a
-- conversation binding is only meaningful alongside its instance default.
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS channel_instance_defaults (
--       instance_name TEXT PRIMARY KEY,
--       agent_name    TEXT NOT NULL,
--       bound_at      TEXT NOT NULL DEFAULT (datetime('now')),
--       bound_by      TEXT
--   );
--   CREATE TABLE IF NOT EXISTS conversation_bindings (
--       instance_name   TEXT NOT NULL,
--       conversation_id TEXT NOT NULL,
--       agent_name      TEXT NOT NULL,
--       bound_at        TEXT NOT NULL DEFAULT (datetime('now')),
--       bound_by        TEXT,
--       PRIMARY KEY (instance_name, conversation_id)
--   );
--
-- `conversation_bindings` has a COMPOSITE primary key. It is reproduced as a
-- UNIQUE composite index, the same treatment 018_group_roster gives its own
-- three-column key — without it a second binding for one conversation would be
-- accepted and inbound dispatch would stop being deterministic, which is the
-- entire point of the table.

DEFINE TABLE IF NOT EXISTS channel_instance_defaults SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS instance_name ON channel_instance_defaults TYPE string;
DEFINE FIELD IF NOT EXISTS agent_name    ON channel_instance_defaults TYPE string;
DEFINE FIELD IF NOT EXISTS bound_at      ON channel_instance_defaults TYPE string;
DEFINE FIELD IF NOT EXISTS bound_by      ON channel_instance_defaults TYPE option<string>;

DEFINE INDEX IF NOT EXISTS idx_channel_instance_defaults_pk
    ON channel_instance_defaults FIELDS instance_name UNIQUE;

DEFINE TABLE IF NOT EXISTS conversation_bindings SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS instance_name   ON conversation_bindings TYPE string;
DEFINE FIELD IF NOT EXISTS conversation_id ON conversation_bindings TYPE string;
DEFINE FIELD IF NOT EXISTS agent_name      ON conversation_bindings TYPE string;
DEFINE FIELD IF NOT EXISTS bound_at        ON conversation_bindings TYPE string;
DEFINE FIELD IF NOT EXISTS bound_by        ON conversation_bindings TYPE option<string>;

DEFINE INDEX IF NOT EXISTS idx_conversation_bindings_pk
    ON conversation_bindings FIELDS instance_name, conversation_id UNIQUE;
