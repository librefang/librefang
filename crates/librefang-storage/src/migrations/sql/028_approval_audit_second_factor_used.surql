-- Migration v28: approval_audit table + second_factor_used field (backfill).
-- Mirrors upstream SQLite v38 (#4874): backfills approval_audit.second_factor_used
-- for upgraded DBs that pre-date the TOTP second-factor patch (#2131).
-- Creates the full table idempotently; the critical addition is second_factor_used.
DEFINE TABLE IF NOT EXISTS approval_audit SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS id                 ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS request_id         ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS agent_id           ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS tool_name          ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS description        ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS action_summary     ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS risk_level         ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS decision           ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS decided_by         ON approval_audit TYPE option<string>;
DEFINE FIELD IF NOT EXISTS decided_at         ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS requested_at       ON approval_audit TYPE string;
DEFINE FIELD IF NOT EXISTS feedback           ON approval_audit TYPE option<string>;
DEFINE FIELD IF NOT EXISTS second_factor_used ON approval_audit TYPE int VALUE $value OR 0;

DEFINE INDEX IF NOT EXISTS idx_approval_audit_agent   ON approval_audit FIELDS agent_id;
DEFINE INDEX IF NOT EXISTS idx_approval_audit_decided ON approval_audit FIELDS decided_at;
