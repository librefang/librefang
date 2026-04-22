-- Phase 6 / migration v2: persistent hook traces.
--
-- Mirrors the rusqlite `hook_traces` table layout. `id` is a Surreal record
-- id we generate from a monotonic ULID-style stringification; `started_at`
-- is stored as ISO-8601 string for the same datetime-coercion reason as the
-- audit chain.

DEFINE TABLE IF NOT EXISTS hook_traces SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS trace_id        ON hook_traces TYPE string;
DEFINE FIELD IF NOT EXISTS correlation_id  ON hook_traces TYPE string;
DEFINE FIELD IF NOT EXISTS plugin          ON hook_traces TYPE string;
DEFINE FIELD IF NOT EXISTS hook            ON hook_traces TYPE string;
DEFINE FIELD IF NOT EXISTS started_at      ON hook_traces TYPE string;
DEFINE FIELD IF NOT EXISTS started_at_ms   ON hook_traces TYPE int;
DEFINE FIELD IF NOT EXISTS elapsed_ms      ON hook_traces TYPE int;
DEFINE FIELD IF NOT EXISTS success         ON hook_traces TYPE bool;
DEFINE FIELD IF NOT EXISTS error           ON hook_traces TYPE option<string>;
-- Stored as JSON-encoded strings to match the rusqlite layout exactly so
-- the migration tool can copy rows verbatim.
DEFINE FIELD IF NOT EXISTS input_preview   ON hook_traces TYPE option<string>;
DEFINE FIELD IF NOT EXISTS output_preview  ON hook_traces TYPE option<string>;

DEFINE INDEX IF NOT EXISTS hook_traces_started_at_idx
    ON hook_traces COLUMNS started_at_ms;
DEFINE INDEX IF NOT EXISTS hook_traces_plugin_hook_idx
    ON hook_traces COLUMNS plugin, hook;
DEFINE INDEX IF NOT EXISTS hook_traces_trace_id_idx
    ON hook_traces COLUMNS trace_id;
DEFINE INDEX IF NOT EXISTS hook_traces_correlation_id_idx
    ON hook_traces COLUMNS correlation_id;
