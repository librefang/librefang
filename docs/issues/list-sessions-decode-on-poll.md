# [Critical] Dashboard 5s `list_sessions()` poll decodes every session's full rmp blob to count rows

**Severity:** Critical
**Domain:** Performance

## Location

`crates/librefang-api/src/routes/config.rs:3031`

```rust
substrate.list_sessions().map(|s| s.len())
```

## Problem

`list_sessions()` returns `Vec<Session>` — each row's full rmp-encoded message history is decoded just to compute `.len()`. The dashboard polls this every 5 seconds (with `refetchIntervalInBackground: false`, so foreground-only — but every active operator hits it constantly).

At 100 sessions × 200 KB each, the daemon burns **~20 MB/s of needless decode work** plus the rmp deserializer's allocation pressure, for what is morphologically a `SELECT COUNT(*)`.

## Fix

Use the existing `count_sessions()` (indexed `SELECT COUNT(*)`):

```rust
substrate.count_sessions()
```

## Tests

- Bench (`criterion` or ad-hoc) showing the swap reduces CPU/alloc on the dashboard hot path by orders of magnitude with N≥50 sessions.
- Snapshot test asserting the route still returns the correct count.
