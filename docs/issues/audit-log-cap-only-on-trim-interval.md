# `AuditLog::record` only enforces the capacity at the trim interval; between trims, growth is unbounded

**Severity:** Low
**Category:** DoS / resource exhaustion
**Labels:** `dos`, `memory`, `low`
**Verification (re-audit 2026-05-18): DISPUTED.** `crates/librefang-runtime-audit/src/lib.rs:29` defines a hard `MAX_AUDIT_ENTRIES = 10_000` constant ceiling, and `record_with_context` (`:684-692`) enforces it on **every push** — not only at `trim()`. The audit's "growth is unbounded between trims" premise is wrong. The configurable `max_in_memory_entries` is trim-only, but the hard ceiling caps memory regardless. Close, or rewrite the recommendation to address the `max_in_memory_entries`-vs-`MAX_AUDIT_ENTRIES` mismatch.

## Affected files
- `crates/librefang-runtime-audit/src/lib.rs:498-560, 886-1007`

## Description

`record_with_context` unconditionally pushes to `self.entries` (`Vec<AuditEntry>` under `std::sync::Mutex`) and writes SQLite. The `max_in_memory_entries` cap is checked **only** inside `trim()`, and `trim()` runs on `trim_interval_secs` (default **3600s** = 1 hour).

Between two trims, a single tool-invocation or LLM-call storm can push the in-memory log into the multi-GiB range. Each `AuditEntry` contains `detail: String` (typically the full tool input) + `outcome: String`.

## Recommendation

Add a soft cap inside `record_with_context`:

```rust
if entries.len() > max_in_memory_entries * 3 / 2 {
    // reuse trim()'s chain-anchor-aware front-eviction logic
    drop_oldest_with_anchor(&mut entries);
}
```

This pins the memory ceiling at 1.5× the configured value rather than depending on the periodic task.
