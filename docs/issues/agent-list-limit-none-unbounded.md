# `AgentListQuery` returns the full unpaginated list when `limit=None`

**Severity:** Medium
**Category:** Input validation
**Labels:** `dos`, `pagination`, `medium`

## Affected files
- `crates/librefang-api/src/routes/agents.rs:1016-1023`
- Similar: `crates/librefang-api/src/routes/types.rs:686-691` (`PaginationQuery.paginate`)
- Correct pattern: `crates/librefang-api/src/routes/audit.rs:239-242`

## Description

```rust
let limit = params.limit.map(|l| l.min(500));   // clamps only when Some
agents.into_iter().skip(offset).collect()       // None → returns everything
```

A multi-thousand-agent deployment triggers memory + JSON-serialization DoS. `audit.rs:239-242`'s `unwrap_or(DEFAULT).min(MAX)` is the correct shape.

## Recommendation

```rust
let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
```

Never allow "None → unpaginated." Apply the same change to the `PaginationQuery::paginate` helper.

Regression test: a list-endpoint request without `limit` returns at most `DEFAULT_LIMIT` rows.
