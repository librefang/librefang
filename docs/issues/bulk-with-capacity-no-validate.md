# Bulk endpoints use `Vec::with_capacity(req.ids.len())` without going through `validate_bulk_size`

**Severity:** Low
**Category:** DoS / resource exhaustion
**Labels:** `dos`, `bulk`, `low`

## Affected files
- `crates/librefang-api/src/routes/approvals.rs:720`
- `crates/librefang-api/src/routes/workflows.rs:503`
- `crates/librefang-api/src/routes/users.rs:505, 535`
- Existing helper: `crates/librefang-api/src/routes/agents.rs:458` (`validate_bulk_size`)

## Description

Each endpoint jumps straight to `Vec::with_capacity(body.ids.len())` / `Vec::with_capacity(arr.len())` with no `validate_bulk_size` pre-check. The 8 MiB global body limit is an absolute cap, but an attacker can still craft an array of empty strings to make `with_capacity` pre-allocate millions of entries.

## Recommendation

Add at the top of every bulk handler:

```rust
validate_bulk_size(body.ids.len(), BULK_LIMIT)?;
```

The helper already exists (`agents.rs:458`); reuse it.
