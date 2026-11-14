-- Migration v27: workflow_runs table for persistent workflow state.
-- Mirrors upstream SQLite v37 (migrate_v37, #4849/#3335).
-- Replaces the tmp+rename workflow_runs.json that lost state on shutdown.
DEFINE TABLE IF NOT EXISTS workflow_runs SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS id                   ON workflow_runs TYPE string;
DEFINE FIELD IF NOT EXISTS workflow_id          ON workflow_runs TYPE string;
DEFINE FIELD IF NOT EXISTS workflow_name        ON workflow_runs TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS state                ON workflow_runs TYPE string
    ASSERT $value IN ['pending', 'running', 'paused', 'completed', 'failed'];
DEFINE FIELD IF NOT EXISTS input                ON workflow_runs TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS output               ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error                ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS resume_token         ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS pause_reason         ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS paused_at            ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS paused_step_index    ON workflow_runs TYPE option<int>;
DEFINE FIELD IF NOT EXISTS paused_variables     ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS paused_current_input ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS step_results         ON workflow_runs TYPE string VALUE $value OR '[]';
DEFINE FIELD IF NOT EXISTS started_at           ON workflow_runs TYPE string;
DEFINE FIELD IF NOT EXISTS completed_at         ON workflow_runs TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at           ON workflow_runs TYPE string;

DEFINE INDEX IF NOT EXISTS idx_workflow_runs_state
    ON workflow_runs FIELDS state;
DEFINE INDEX IF NOT EXISTS idx_workflow_runs_workflow_id
    ON workflow_runs FIELDS workflow_id;
DEFINE INDEX IF NOT EXISTS idx_workflow_runs_started_at
    ON workflow_runs FIELDS started_at;
