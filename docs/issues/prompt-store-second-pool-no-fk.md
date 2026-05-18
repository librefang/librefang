# `PromptStore::new_with_path` opens an independent connection pool without `foreign_keys=ON`

**Severity:** Medium
**Category:** SQLite and migration-layer data integrity
**Labels:** `data-integrity`, `sqlite`, `medium`

## Affected files
- `crates/librefang-memory/src/prompt.rs:113-130`
- Reference: `crates/librefang-memory/src/substrate.rs:100-105`

## Description

The substrate pool sets `PRAGMA foreign_keys=ON`. **SQLite's `foreign_keys` is per-connection, not per-database.**

`PromptStore::new_with_path` opens a **second** pool on the same file but only sets `journal_mode`, `busy_timeout`, `cache_size`, and `mmap_size` — `foreign_keys=ON` is missing.

Consequence: writes through this connection **silently bypass** the FK constraints declared by `migrate_v13` on `prompt_experiments` / `experiment_variants` / `experiment_metrics`.

## Recommendation

Reuse the full PRAGMA set from `substrate.rs:100-105` in `PromptStore::new_with_path`, especially `foreign_keys=ON` and `synchronous=NORMAL`.

Even better: extract a shared helper:

```rust
pub(crate) fn apply_pragmas(conn: &mut Connection) -> Result<()> { ... }
```

placed in the substrate module so any future "second pool" inherits it automatically.
