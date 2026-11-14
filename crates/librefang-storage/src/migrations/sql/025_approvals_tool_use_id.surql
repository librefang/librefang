-- Migration v25: add tool_use_id to pending_approvals
-- Mirrors upstream SQLite ALTER TABLE pending_approvals ADD COLUMN tool_use_id TEXT.
-- Links the approval request back to the streaming ToolCall card (ACP adapter, #3313).
-- None for the synchronous request_approval path and for pre-existing rows.
DEFINE FIELD IF NOT EXISTS tool_use_id ON pending_approvals TYPE option<string>;
