# Multi-Tenant Migration Runbook

## Memory Store v19

### Scope

This runbook covers the `librefang-memory` SQLite schema migration that adds a
physical `memories.account_id` column for tenant-scoped queries.

### What Changed

- schema version `19` adds `memories.account_id`
- new column is `TEXT NOT NULL DEFAULT 'default'`
- legacy rows backfill from `metadata.account_id` when present
- rows without legacy tenant metadata normalize to `default`
- account-scoped memory queries now use the physical column instead of JSON extraction

### Expected Post-Migration State

Run these checks against the migrated SQLite database:

```sql
PRAGMA table_info(memories);
SELECT COUNT(*) FROM memories WHERE account_id IS NULL OR trim(account_id) = '';
SELECT account_id, COUNT(*) FROM memories GROUP BY account_id ORDER BY COUNT(*) DESC;
```

Expected results:

- `account_id` exists on `memories`
- `account_id` is `NOT NULL`
- the null/blank count is `0`
- pre-migration rows without tenant metadata appear under `default`

### Local Development Caveat

If a developer previously ran an unshipped local draft that created
`memories.account_id` as nullable, rebuild that local DB or normalize the column
before relying on tenant-scoped memory queries.

Safe options:

1. delete the local dev DB and let LibreFang recreate it
2. run a one-time repair:

```sql
UPDATE memories
SET account_id = 'default'
WHERE account_id IS NULL OR trim(account_id) = '';
```

Then verify:

```sql
SELECT COUNT(*) FROM memories WHERE account_id IS NULL OR trim(account_id) = '';
```

### Verification Commands

```bash
cargo test -p librefang-memory test_migration_v19_adds_memories_account_column -- --nocapture
cargo test -p librefang-memory test_migration_v19_backfills_memories_account_from_metadata -- --nocapture
cargo test -p librefang-memory test_migration_v19_defaults_missing_memories_account_id -- --nocapture
cargo test -p librefang-memory test_count_by_account_uses_physical_account_column -- --nocapture
cargo test -p librefang-memory test_update_content_syncs_account_id_column -- --nocapture
```
