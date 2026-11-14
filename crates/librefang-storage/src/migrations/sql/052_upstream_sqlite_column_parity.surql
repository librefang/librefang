-- Migration v52: close the column and index gaps found by a column-by-column audit of the SQLite schema against this ladder.
-- Every statement mirrors a SQLite column, index or CHECK that already shipped in an earlier upstream step but never reached a .surql file.
-- The tables are SCHEMAFULL, so until now each of these columns was silently dropped on write.
--
-- Upstream steps mirrored:
--   v2:  ALTER TABLE task_queue ADD COLUMN title / description / created_by / result TEXT DEFAULT ''
--   v8:  CREATE INDEX idx_audit_timestamp ON audit_entries(timestamp); CREATE INDEX idx_audit_action ON audit_entries(action)
--   v16: ALTER TABLE sessions ADD COLUMN peer_id TEXT DEFAULT NULL; CREATE INDEX idx_sessions_peer ON sessions(agent_id, peer_id)
--   v19: CREATE INDEX idx_usage_provider_time ON usage_events(provider, timestamp)
--   v20: ALTER TABLE task_queue ADD COLUMN claimed_at TEXT DEFAULT NULL; CREATE INDEX idx_task_status_claimed_at ON task_queue(status, claimed_at)
--   v21: ALTER TABLE task_queue ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0
--   v22: CREATE INDEX idx_audit_channel ON audit_entries(channel)
--   v23: CREATE INDEX idx_usage_channel_time ON usage_events(channel, timestamp)
--   v49 (#7714): ALTER TABLE workflow_runs ADD COLUMN owner_agent_id TEXT DEFAULT NULL; ALTER TABLE usage_events ADD COLUMN billed_agent_id TEXT DEFAULT NULL
--   v52 (#7086): ALTER TABLE group_roster ADD COLUMN source TEXT NOT NULL DEFAULT 'observed'
--   workflow_runs.state accepts 'cancelled', which the kernel writes for WorkflowRunState::Cancelled (workflow_run_to_row in crates/librefang-kernel/src/workflow.rs) but the v37 SQLite CHECK omits
--   out-of-ladder: crates/librefang-runtime/src/trace_store.rs ALTER TABLE hook_traces ADD COLUMN annotations TEXT
--
-- `hook_traces.annotations` is JSON text, as `input_preview` / `output_preview` already are on this table.
-- `group_roster.source` is 'observed' or 'enumerated'; SQLite has no CHECK on it, so neither does this field.
--
-- SurrealDB 4 refuses, on a populated SCHEMAFULL table in a compatibility-level-4 database, a field definition that does not accept every value already stored (a new non-optional field, a narrowed TYPE, a changed ASSERT).
-- Each such definition below therefore says what existing rows hold with MIGRATE EXISTING ROWS, the same value SQLite's ALTER TABLE ... DEFAULT backfills.
-- On an empty table, and on a database still at compatibility level 3, the clause is accepted and has nothing to migrate.

DEFINE FIELD IF NOT EXISTS peer_id ON sessions TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_sessions_peer ON sessions FIELDS agent_id, peer_id;

DEFINE FIELD IF NOT EXISTS title       ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS description ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_by  ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS result      ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS claimed_at  ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS retry_count ON task_queue TYPE int VALUE $value ?? 0
    MIGRATE EXISTING ROWS AS task_queue_retry_count_v52 USING $value ?? 0 FALLBACK 0;
DEFINE INDEX IF NOT EXISTS idx_task_status_claimed_at ON task_queue FIELDS status, claimed_at;

DEFINE FIELD IF NOT EXISTS billed_agent_id ON usage_events TYPE option<string>;
DEFINE INDEX IF NOT EXISTS usage_provider_time_idx ON usage_events FIELDS provider, timestamp;
DEFINE INDEX IF NOT EXISTS usage_channel_time_idx  ON usage_events FIELDS channel, timestamp;

DEFINE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_entries FIELDS timestamp;
DEFINE INDEX IF NOT EXISTS idx_audit_action    ON audit_entries FIELDS action;
DEFINE INDEX IF NOT EXISTS idx_audit_channel   ON audit_entries FIELDS channel;

DEFINE FIELD IF NOT EXISTS owner_agent_id ON workflow_runs TYPE option<string>;
-- SurrealDB 4 will not migrate a field an index reads, so `idx_workflow_runs_state` (027) is dropped around the redefinition and restored unchanged.
REMOVE INDEX IF EXISTS idx_workflow_runs_state ON workflow_runs;
DEFINE FIELD OVERWRITE state ON workflow_runs TYPE string
    ASSERT $value IN ['pending', 'running', 'paused', 'completed', 'failed', 'cancelled']
    MIGRATE EXISTING ROWS AS workflow_runs_state_cancelled_v52 USING $value FALLBACK 'failed';
DEFINE INDEX IF NOT EXISTS idx_workflow_runs_state ON workflow_runs FIELDS state;

DEFINE FIELD IF NOT EXISTS source ON group_roster TYPE string VALUE $value ?? 'observed'
    MIGRATE EXISTING ROWS AS group_roster_source_v52 USING $value ?? 'observed' FALLBACK 'observed';

DEFINE FIELD IF NOT EXISTS annotations ON hook_traces TYPE option<string>;

-- `prompt_experiments.traffic_split` is `PromptExperiment::traffic_split: Vec<u8>`, which SQLite stores as a JSON array ("[50,50]").
-- 010 typed it `option<object>`, which cannot hold a real split; a writer could only store an `{}` placeholder.
-- No stored row can hold an array (010 rejected them), so every existing value is a placeholder that carries no split and migrates to NONE.
-- A stored `{}` in `success_criteria` would decode as all-false criteria instead of `SuccessCriteria::default()`, so it becomes NONE too; the field's type is unchanged.
UPDATE prompt_experiments SET success_criteria = NONE WHERE success_criteria = {};
DEFINE FIELD OVERWRITE traffic_split ON prompt_experiments TYPE option<array<int>>
    MIGRATE EXISTING ROWS AS prompt_experiments_traffic_split_v52 USING NONE FALLBACK NONE;
