# [Low] `agent_concurrency_for` get-then-insert is not atomic

**Severity:** Low · **Domain:** Concurrency · **Source:** `audit-03-concurrency.md`

**Location:** `crates/librefang-kernel/src/kernel/accessors.rs:945-988`

**Problem:** Two concurrent first-callers both miss in `get()`, both compute `resolved_cap`, both race to `or_insert_with`. Semantically idempotent today, but does redundant work and emits a duplicate `tracing::warn!` on clamp.

**Fix:** Move manifest read inside `entry().or_insert_with(...)`.
