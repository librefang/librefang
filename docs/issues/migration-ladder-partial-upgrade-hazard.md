# [Medium] SQLite migration ladder hardening — partial upgrade, UUID CHECK, identifier interpolation, nonce hygiene

**Severity:** Medium
**Category:** Data integrity · SQL
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | Medium | Each `migrate_vN` runs in its own tx; a mid-ladder crash leaves a partial upgrade where `pragma user_version` and the `migrations` table can disagree |
| UUID CHECK | Medium | `agents.id` / `sessions.id` and similar UUID primary keys have no DB-side CHECK; an external write of a non-UUID value blows up Rust's `from_str` later |
| identifier interpolation | Low | The `column_exists` PRAGMA + `ALTER TABLE` helper uses `format!` to splice identifiers without an allowlist |
| nonce hygiene | Low | `oauth_used_nonces.nonce_hash` has no `CHECK (length=64)`; the boot-time prune sweep is not confirmed to be wired up |

## Affected files

- `crates/librefang-memory/src/migration.rs:30-39, 41-180` (ladder + `run_step!`)
- `crates/librefang-memory/src/migration.rs:188-198, 341` (`column_exists`, `ALTER TABLE`)
- `crates/librefang-memory/src/migration.rs:210-217, 228-236` (UUID tables)
- `crates/librefang-memory/src/migration.rs:961-976` (`oauth_used_nonces`)
- Also-applies-to: `totp_used_codes`

## Why merged

All four are migration / schema hygiene items concentrated in `migration.rs`. A single consistency sweep is more economical than one-off PRs.

## Combined fix plan

1. **Boot-time invariant check (this)**: at startup, verify `MAX(migrations.version) == pragma user_version`; fail loud on mismatch:
   ```rust
   let max_mig: u32 = conn.query_row("SELECT IFNULL(MAX(version),0) FROM migrations", [], |r| r.get(0))?;
   let user_ver: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
   if max_mig != user_ver { return Err(MigrationError::Inconsistent { max_mig, user_ver }); }
   ```
   Keep per-step txs (whole-ladder rollback is worse), but document the partial-upgrade contract explicitly.
2. **UUID-shape CHECK (UUID CHECK)**: a new migration adds:
   ```sql
   CHECK (length(id) = 36 AND id GLOB '*-*-*-*-*')
   ```
   to every UUID primary key (`agents`, `sessions`, `memories`, `entities`, `relations`, `task_queue`, `usage_events`, `audit_entries`, `pending_approvals`, `idempotency_keys`, `workflow_runs`).
3. **Identifier allowlist (identifier interpolation)**: `column_exists` and `ALTER TABLE … ADD COLUMN` helpers validate `^[a-zA-Z_]\w*$` before splicing, otherwise return `Err(MigrationError::InvalidIdentifier)`.
4. **Nonce / TOTP length CHECK + prune (nonce hygiene)**:
   ```sql
   CHECK (length(nonce_hash) = 64 AND nonce_hash GLOB '[0-9a-f]*')
   ```
   Additionally, verify the boot scheduler actually enqueues the prune sweep — if it's not hooked up, that's a real bug; enqueue automatically.

## Tests

- (this) Panic inside `migrate_vN`; on restart, the invariant check trips.
- (UUID CHECK) `INSERT INTO agents(id) VALUES ('not-a-uuid')` → CHECK violation.
- (identifier interpolation) `column_exists(conn, "users; DROP TABLE")` → returns `InvalidIdentifier`.
- (nonce hygiene) 1 hour + 1s after startup, assert rows in `oauth_used_nonces` older than 1h are pruned.
