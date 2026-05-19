# [Medium] Per-trigger `session_mode_override = New` is throttled by the manifest's `Persistent` clamp

**Severity:** Medium · **Domain:** Concurrency · **Source:** `audit-03-concurrency.md`

## Location
- `crates/librefang-kernel/src/kernel/triggers_and_workflow.rs:338-350` (override site)
- `crates/librefang-kernel/src/kernel/accessors.rs:959-982` (cap clamp)

## Problem
Per-trigger `New` override mints fresh sessions (correct), but `agent_concurrency_for` was already clamped to 1 because the **manifest** default is `Persistent`. CLAUDE.md documents this as intentional — recorded here so the trade-off is visible.

Effect: a trigger that wants parallelism cannot get it unless the agent's manifest default is also `New`. This may surprise users who think per-trigger overrides give full control.

## Fix
Decision needed:
- **Keep as-is:** document more prominently in `agent.toml` / trigger config.
- **Relax:** let per-trigger override the cap when override is `New`. Risk: prompt-cache reuse is harder to reason about.

## Tests
- Document the current behavior in `docs/architecture/trigger-dispatch-concurrency.md` (already exists, may need updating).
