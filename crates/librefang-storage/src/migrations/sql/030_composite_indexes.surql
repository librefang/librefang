-- Migration v30: Composite indexes for per-agent recency hot-paths.
-- Mirrors SQLite migration v41 (upstream #5641).
--
-- `sessions(agent_id, updated_at)`: used by count_agent_sessions_touched_since
-- (concurrent-trigger admission check, runs on every cron/trigger fire).
-- The single-column sessions_agent_idx (v5) covers agent_id filters but forces
-- a secondary sort step on updated_at; the composite eliminates the sort for
-- the hot recency path.
--
-- `audit_entries(agent_id, timestamp)`: the existing audit_entries_agent_idx
-- (v1) is single-column on agent_id; per-agent audit-history queries still pay
-- a sort on timestamp. The composite lets the planner stream rows in
-- (agent_id, timestamp) order without a sort step.
--
-- Both `IF NOT EXISTS` — idempotent on fresh installs and re-applied migrations.

DEFINE INDEX IF NOT EXISTS idx_sessions_agent_updated
    ON sessions COLUMNS agent_id, updated_at;

DEFINE INDEX IF NOT EXISTS idx_audit_agent_timestamp
    ON audit_entries COLUMNS agent_id, timestamp;
