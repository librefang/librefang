# `TraceStore::insert` holds a `std::sync::Mutex` across SQLite I/O on a tokio worker

**Severity:** Medium
**Category:** Concurrency, races, deadlocks
**Labels:** `concurrency`, `performance`, `medium`

## Affected files
- `crates/librefang-runtime/src/trace_store.rs:55, 64-96`
- Callers: `crates/librefang-runtime/src/context_engine/scriptable/mod.rs:705-722` → `engine.rs:853, 874, 895` (hooks run inside `tokio::spawn`)

## Description

`insert()`:

- Holds `std::sync::Mutex<Connection>`;
- Executes `INSERT` + `DELETE … WHERE id NOT IN (SELECT … LIMIT 10000)` (O(N) full-table scan-and-delete);
- **No `spawn_blocking`**.

WAL-mode steady-state writes are fast, but the prune is linear. Under a hook storm, each call blocks a tokio worker. `registry_sync.rs:37` annotates the equivalent call with "caller goes through `spawn_blocking`"; this site is missing it.

## Recommendation

1. Wrap `store.insert(...)` in `tokio::task::spawn_blocking`; or
2. Push traces into a channel; drain via a dedicated `spawn_blocking` writer;
3. Don't run the prune on every insert — trigger every N inserts or on a timer, amortizing the full-table scan.
