# [High] Raw `rusqlite::Error::to_string()` propagates to clients in non-memory routes

**Severity:** High · **Domain:** Error handling · **Source:** `audit-04-error-handling.md`

## Location
- Source: `crates/librefang-memory/src/substrate.rs:368, 625, 664, 677, 839, 872, 906, 921`
  ```rust
  .map_err(|e| LibreFangError::Internal(e.to_string()))
  ```
- Leaking to client: `crates/librefang-api/src/routes/agents.rs:1147-1149, 1268-1270, 5263-5268` (and more — every route that echoes `e.to_string()`)
- Correct pattern (only path that scrubs): `crates/librefang-api/src/routes/memory.rs:198-215` (`MemoryRouteError`)

## Problem
Strings like `"no such column: foo"`, `"UNIQUE constraint failed: agents.id"`, `"database is locked"` flow into `LibreFangError::Internal`, then most routes echo `e.to_string()` directly to the client. This leaks schema details to anyone who can trigger an internal error.

## Fix
Lift `MemoryRouteError::Internal` scrubbing into a workspace-wide helper:

```rust
// crates/librefang-api/src/error.rs
pub fn internal_err(e: impl std::fmt::Display) -> HttpError {
    let full = e.to_string();
    tracing::error!(error = %full, "internal error");
    HttpError::internal("Internal server error")
}
```

Replace every `HttpError::internal(e.to_string())` with `internal_err(e)`. Audit `rg 'HttpError::internal\\(.*\\.to_string'`.

## Tests
- Force a constraint violation (insert duplicate agent id) → 500 with body `{"error":"Internal server error"}`, **not** the SQL message.
- Server logs (tracing capture) contain the full SQL message for ops.
