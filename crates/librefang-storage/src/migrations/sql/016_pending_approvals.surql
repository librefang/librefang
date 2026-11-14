-- Migration v16: pending_approvals table for cross-restart approval persistence (mirrors SQLite V26, upstream #3611).
-- Restored entries surface in the dashboard/API as pending items requiring operator action
-- when the daemon restarts with in-flight approvals.

DEFINE TABLE IF NOT EXISTS pending_approvals SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS id         ON pending_approvals TYPE string;
DEFINE FIELD IF NOT EXISTS agent_id   ON pending_approvals TYPE string;
DEFINE FIELD IF NOT EXISTS session_id ON pending_approvals TYPE option<string>;
DEFINE FIELD IF NOT EXISTS tool_name  ON pending_approvals TYPE string;
DEFINE FIELD IF NOT EXISTS tool_input ON pending_approvals TYPE string VALUE $value OR '';
DEFINE FIELD IF NOT EXISTS created_at ON pending_approvals TYPE int;
DEFINE FIELD IF NOT EXISTS expires_at ON pending_approvals TYPE option<int>;

DEFINE INDEX IF NOT EXISTS idx_pending_approvals_agent ON pending_approvals FIELDS agent_id;
