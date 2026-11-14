-- Migration v34: workflow_runs.total_steps.
-- Mirrors upstream SQLite v56 (migrate_v56, #7776) and its forward-compat
-- re-run v59 (#8318): `ALTER TABLE workflow_runs ADD COLUMN total_steps
-- INTEGER NOT NULL DEFAULT 0`.
--
-- Added here rather than edited into 027_workflow_runs.surql because applied
-- migrations are SHA256 checksum-locked; editing a landed file is a boot-time
-- hard failure (MigrationError::ChecksumDrift).
--
-- Upstream stores the column as INTEGER NOT NULL DEFAULT 0 and reads it into
-- `WorkflowRunRow.total_steps: i64`, so the SurrealDB field is `int` with the
-- same zero default for rows written before the field existed.

DEFINE FIELD IF NOT EXISTS total_steps ON workflow_runs TYPE int VALUE $value OR 0;
