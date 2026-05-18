# SQLite database files created with world-readable permissions (no `chmod 0600`)

**Severity:** High
**Category:** SQLite and migration-layer data integrity
**Labels:** `security`, `data-leak`, `filesystem`, `high`

## Affected files
- `crates/librefang-kernel/src/kernel/boot.rs:181-209`
- `crates/librefang-memory/src/substrate.rs:82-116`

## Description

`librefang.db`, `-wal`, and `-shm` are created by `SqliteConnectionManager::file(db_path)` with permissions following the current umask (typically `0644`) — **any process under the same UID can read them**.

The DB contains:

- session history (raw user prompts and LLM replies);
- `kv_store` agent state (potentially cleartext credentials);
- `audit_entries`;
- `oauth_used_nonces`, `totp_used_codes`;
- `paired_devices.api_key_hash`.

On shared hosts (dev machines, CI runners, multi-user dev boxes), every local user can `cat ~/.librefang/librefang.db`.

`librefang-migrate/src/openclaw.rs:655` already hardens `.env` writes with `0o600` — the project knows the pattern; it just isn't applied to SQLite.

## Recommendation

In `boot.rs`, after `create_dir_all(&config.data_dir)`:

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&config.data_dir,
        std::fs::Permissions::from_mode(0o700))?;
}
```

After `MemorySubstrate::open_with_pool_size` returns, call `set_permissions(0o600)` on the three files. The same rule applies to any test / CLI path that opens a file-backed DB outside boot.

Regression test: `#[cfg(unix)] tokio::test` asserting `metadata.permissions().mode() & 0o777 == 0o600`.
