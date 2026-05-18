# [High] `agent_budget_ranking` deep-clones every `AgentEntry`

**Severity:** High · **Domain:** Performance · **Source:** `audit-05-performance.md`

## Location
`crates/librefang-api/src/routes/budget.rs:750`

## Problem
Uses `registry.list()` (returns owned `Vec<AgentEntry>`) instead of `registry.list_arcs()` (returns `Vec<Arc<AgentEntry>>`). Each `AgentEntry` contains the full `AgentManifest` — at scale, several KB per agent — so a ranking over 200 agents copies ~600 KB on every dashboard refresh.

This is exactly the regression #3569 fixed for `GET /api/agents` — the budget ranking endpoint was missed.

## Fix
```rust
let agents = state.kernel.registry.list_arcs();
```
And update downstream code to work with `Arc<AgentEntry>` (deref is transparent for read-only access).

## Tests
- Bench: `agent_budget_ranking` with 200 agents drops to < 5 ms wall time and < 1 KB heap delta.
