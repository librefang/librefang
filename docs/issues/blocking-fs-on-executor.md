# [High] Blocking `std::fs` + walkdir on axum executor

**Severity:** High · **Domain:** Performance · **Source:** `audit-05-performance.md`

## Location
- `crates/librefang-api/src/routes/backup.rs:46-189` (`create_backup`)
- `crates/librefang-api/src/routes/budget.rs:580-651` (`persist_budget`)

## Problem
Both functions perform `std::fs` operations + `walkdir` traversal **directly on the axum/tokio worker thread** that's serving the HTTP request. A large backup can stall the executor for seconds, blocking all concurrent requests on that worker.

`create_backup` walks the entire `~/.librefang/` tree; `persist_budget` does fsync-heavy small writes on every budget update.

## Fix
Wrap with `tokio::task::spawn_blocking`:
```rust
let result = tokio::task::spawn_blocking(move || {
    create_backup_blocking(&home)
}).await??;
```

Or switch to `tokio::fs` for the small-file writes in `persist_budget`.

## Tests
- Smoke: while a `create_backup` is in flight, an unrelated `GET /api/health` returns within < 50 ms.
