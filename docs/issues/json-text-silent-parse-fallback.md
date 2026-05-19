# JSON-in-TEXT columns silently fall back on parse failure → corruption is disguised as "no data"

**Severity:** Medium
**Category:** SQLite and migration-layer data integrity
**Labels:** `data-integrity`, `parsing`, `medium`

## Affected files
- `crates/librefang-memory/src/semantic.rs:346, 555, 991`
- `crates/librefang-memory/src/knowledge.rs:271, 298`
- `crates/librefang-memory/src/structured.rs:261-266`

## Description

```rust
serde_json::from_str(&meta_str).unwrap_or_default()
```

A corrupted `properties` / `metadata` row (manual SQL edit, pre-#3451 FTS transaction bug, upstream `serde(rename)` drift) → silently returns `HashMap::default()` — **the caller cannot distinguish "no metadata" from "metadata destroyed."**

`structured.rs:261` is worse: on JSON parse failure it falls back to "treat the blob as a UTF-8 string," fabricating a `Value::String` out of thin air. `list_kv` runs this path → corrupted rows get injected into the agent's LLM context.

## Recommendation

Change the fallback to:

```rust
match serde_json::from_str::<HashMap<_, _>>(&meta_str) {
    Ok(m) => m,
    Err(e) => {
        error!(row_id, table = %T, error = %e, "corrupt JSON in TEXT column");
        return Err(LibreFangError::serialization(e));
        // or: continue; to skip the row
    }
}
```

**Never** substitute a default for "the DB returned bytes I can't decode." Corruption disguised as success is an audit nightmare.
