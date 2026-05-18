# `cleanup_orphan_sessions` builds SQL via `format!` instead of parameter binding

**Severity:** Medium (mitigated today by `AgentId = Uuid`; defense-in-depth and style regression)
**Category:** SQLite and migration-layer data integrity
**Labels:** `security`, `sql`, `defense-in-depth`, `medium`

## Affected files
- `crates/librefang-memory/src/session.rs:1289-1305`

## Description

```rust
placeholders.push(format!("'{}'", id.0))
let sql = format!("DELETE FROM sessions WHERE agent_id NOT IN ({in_clause})")
```

Today `id.0` is a `uuid::Uuid`, whose `Display` only emits `[0-9a-f-]` → safe. But:

- This is the **only** site in the substrate that violates the "use `?` binding" rule;
- The moment `AgentId(Uuid)` becomes `AgentId(String)` (say `AgentId::for_hand` namespacing), it silently becomes a SQLi vector;
- No test pins this invariant down.

## Recommendation

```rust
let q = format!(
    "DELETE FROM sessions WHERE agent_id NOT IN ({})",
    std::iter::repeat("?").take(live_agent_ids.len()).collect::<Vec<_>>().join(",")
);
conn.execute(&q, rusqlite::params_from_iter(
    live_agent_ids.iter().map(|id| id.0.to_string())
))?;
```

Test: feed in an id containing `'` and assert no injection (even if the `AgentId` type is relaxed in the future).
