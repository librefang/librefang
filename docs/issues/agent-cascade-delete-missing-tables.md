# Agent delete cascade misses 8 `agent_id`-keyed tables → orphan rows can still authenticate

**Severity:** High
**Category:** SQLite and migration-layer data integrity
**Labels:** `security`, `data-integrity`, `auth`, `high`

## Affected files
- `crates/librefang-memory/src/structured.rs:706-733` (`execute_structured_agent_deletes`)
- `crates/librefang-memory/src/migration.rs` (table schema)

## Description

`execute_structured_agent_deletes` stops after enumerating 14 tables. The following tables also carry `agent_id` columns and are **not** purged:

- `paired_devices` (v7, stores `api_key_hash`) ← **bearer-token replay**
- `pending_approvals` (v26)
- `idempotency_keys` (v34)
- `workflow_runs` (v37, indirect via `workflow_id → agent`)
- Plus `oauth_used_nonces`, which is global and never pruned (see the dedicated issue).

`PRAGMA foreign_keys=ON` is enabled at `substrate.rs:104`, but only `prompt_experiments`, `experiment_variants`, and `experiment_metrics` declare `FOREIGN KEY` clauses. The remaining tables enforce no FK → deleting an agent leaves:

- `paired_devices` rows that continue to authenticate against the old bearer token (**replay against a deleted identity**);
- `pending_approvals` rows that fail-open on restart recovery.

## Recommendation

Append to `execute_structured_agent_deletes`:

```rust
DELETE FROM paired_devices    WHERE agent_id = ?1
DELETE FROM pending_approvals WHERE agent_id = ?1
DELETE FROM idempotency_keys  WHERE agent_id = ?1  -- scope as appropriate
DELETE FROM workflow_runs
   WHERE workflow_id IN (SELECT id FROM workflows WHERE agent_id = ?1)
```

Regression test: a script walks every table in `migration.rs` that has an `agent_id` column; after `remove_agent`, assert each table has zero matching rows.
