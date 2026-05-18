# `sessions.agent_id` lacks an independent index; hot paths fall back to full-table scan

**Severity:** Medium
**Category:** SQLite and migration-layer data integrity
**Labels:** `performance`, `index`, `medium`

## Affected files
- `crates/librefang-memory/src/migration.rs:228-236, 670-696`
- Consumers: `crates/librefang-memory/src/session.rs:544, 605, 667, 776, 792, 1165, 1609, 1799`

## Description

The `sessions` table has no standalone `agent_id` index. The closest is `idx_sessions_peer ON sessions(agent_id, peer_id)` from v16:

- In theory it can prefix-scan `WHERE agent_id = ?`, but the query planner's choice is not stable;
- The index name suggests "filter by peer," so anyone manually hinting in the future is likely to avoid it.

Hot path:

```sql
SELECT COUNT(*) FROM sessions
 WHERE agent_id = ?1 AND updated_at > ?2
```

(`count_agent_sessions_touched_since`, run on every concurrent trigger check); once `sessions` grows past a few thousand rows, it degrades to a full-table scan. `DELETE FROM sessions WHERE agent_id = ?1` (cascade) has the same problem.

## Recommendation

Add a migration:

```sql
CREATE INDEX IF NOT EXISTS idx_sessions_agent_updated
  ON sessions(agent_id, updated_at DESC);
```

Apply the same approach to `audit_entries(agent_id, timestamp)` (v8 only builds a single-column index).
