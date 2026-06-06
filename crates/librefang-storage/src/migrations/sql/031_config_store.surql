-- Migration v31: system-scoped config store (BossFang-exclusive).
--
-- Backs the database-backed configuration store (phase-9-config-store-migration).
-- Runtime-mutable, UI-editable config (MCP server registrations, default model,
-- provider URLs, and the `/api/config/set` allowlisted subset) lives here instead
-- of `config.toml`, so the web UI can persist changes even when `config.toml` is
-- mounted read-only from a Kubernetes ConfigMap.
--
-- This is a NEW BossFang table with NO upstream SQLite equivalent — upstream
-- LibreFang keeps all such config in TOML. Do not expect a matching
-- `CREATE TABLE` in any upstream SQLite migration.
--
-- Distinct from `kv_store` (v6): that table is AGENT-scoped (UNIQUE on
-- (agent_id, key)) and holds per-agent runtime blobs. `config_store` is
-- SYSTEM-scoped (UNIQUE on key alone) and holds daemon configuration.
--
-- Provenance + conflict model (see assessment §1b / plan D3):
--   * `source`       — "bootstrap" (seeded from config.toml defaults) or
--                      "runtime" (written by the UI / API). A bootstrap re-sync
--                      NEVER overwrites a "runtime" row on a mere content change.
--   * `content_hash` — hash of the bootstrap value last seen for this key. The
--                      seed/merge logic compares THIS, never the file mtime
--                      (a ConfigMap-projected file's mtime changes on every
--                      kubelet sync even when content is identical).
--   * `revision`     — operator-bumped bootstrap revision; a strictly-greater
--                      bootstrap revision is the only way a file value overrides
--                      an existing row regardless of source.
--
-- `value` is FLEXIBLE to allow arbitrary JSON config blobs (e.g. a serialized
-- McpServerConfigEntry). `IF NOT EXISTS` throughout — idempotent on fresh
-- installs and re-applied migrations.

DEFINE TABLE IF NOT EXISTS config_store SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS key          ON config_store TYPE string;
DEFINE FIELD IF NOT EXISTS value        ON config_store TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS source       ON config_store TYPE string;
DEFINE FIELD IF NOT EXISTS content_hash ON config_store TYPE string;
DEFINE FIELD IF NOT EXISTS revision     ON config_store TYPE int;
DEFINE FIELD IF NOT EXISTS updated_at   ON config_store TYPE string;

DEFINE INDEX IF NOT EXISTS config_store_key_idx
    ON config_store COLUMNS key UNIQUE;
