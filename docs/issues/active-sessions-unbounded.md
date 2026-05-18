# `active_sessions` is only swept lazily at WS upgrade time; grows unbounded otherwise

**Severity:** Medium
**Category:** DoS / resource exhaustion
**Labels:** `dos`, `memory-leak`, `session`, `medium`
**Verification (re-audit 2026-05-18): FIXED.** `crates/librefang-api/src/server.rs:1791-1802` already runs a 5-minute periodic GC task that `retain`s on `active_sessions`. The audit's "WS upgrade is the only sweep" premise contradicts the current code. Close as obsolete or downgrade to "the GC interval is 5 min — confirm acceptable under high login churn."

## Affected files
- `crates/librefang-api/src/server.rs:931-950, 1029, 1090`
- `crates/librefang-api/src/ws.rs:407-415` (the only sweep point)

## Description

`active_sessions: HashMap<String, SessionToken>` is persisted to disk and reloaded after restart. **The sweep runs only at WebSocket upgrade** via `sessions.retain(...)` — there is no periodic prune.

An attacker that can reach the login endpoint (only per-IP rate-limited) adds a session entry on every successful login that **survives daemon restart**. RAM + disk usage grow as `n_logins × token_size`.

## Recommendation

1. Periodic task (every 60s):

```rust
sessions.retain(|_, st| !is_token_expired(st, DEFAULT_SESSION_TTL_SECS));
// then re-persist
```

2. Cap `MAX_ACTIVE_SESSIONS`; LRU-evict on overflow;
3. Before persisting, dedup expired tokens.
