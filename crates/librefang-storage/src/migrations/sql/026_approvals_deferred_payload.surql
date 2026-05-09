-- Migration v26: add deferred_payload to pending_approvals
-- Mirrors upstream SQLite ALTER TABLE pending_approvals ADD COLUMN deferred_payload BLOB.
-- Stores serialized DeferredApprovalPayload so deferred-path approvals survive daemon restart.
-- Null for the synchronous request_approval path and for pre-existing rows.
DEFINE FIELD IF NOT EXISTS deferred_payload ON pending_approvals TYPE option<bytes>;
