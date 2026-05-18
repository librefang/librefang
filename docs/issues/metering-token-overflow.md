# Metering token computation can be made to overflow `u64` by an LLM-provider response

**Severity:** Medium
**Category:** Panic / error handling
**Labels:** `bug`, `metering`, `overflow`, `medium`

## Affected files
- `crates/librefang-kernel-metering/src/lib.rs:706`

## Description

```rust
input_tokens.saturating_sub(cache_read_input_tokens + cache_creation_input_tokens)
```

The outer `saturating_sub` is fine, but the inner `cache_read + cache_creation` is **not** `saturating_add`. All three fields come from LLM-provider wire data (`UsageInfo`, `u64`). A malicious or buggy provider returning `u64::MAX/2 + 1` in both cache fields produces:

- A panic in debug builds;
- A wrap-around in release → silently produces absurd budget rows;
- Affects `BudgetStatus` and spend-cap enforcement — the input to billing / throttling.

## Recommendation

```rust
cache_read_input_tokens
    .saturating_add(cache_creation_input_tokens)
```

Additionally, at driver-response deserialization, pre-clamp `input_tokens` / `cache_*` to a sane upper bound (e.g. `u32::MAX as u64`).
