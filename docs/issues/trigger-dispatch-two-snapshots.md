# [Medium] Trigger dispatch reads `agent.session_mode` twice from two manifest snapshots

**Severity:** Medium · **Domain:** Concurrency · **Source:** `audit-03-concurrency.md`

## Location
- `crates/librefang-kernel/src/kernel/triggers_and_workflow.rs:338-348` (read A)
- `crates/librefang-kernel/src/kernel/accessors.rs:945-988` (read B — `agent_concurrency_for`)

## Problem
The dispatcher reads `manifest.session_mode` to decide if it needs `Some(SessionId::new())`. Then `agent_concurrency_for` does its own `registry.get` for the cap. The cap is **NOT** invalidated on hot-reload (per CLAUDE.md). So hot-reloading `Persistent → New` makes the dispatcher mint fresh sessions but throttle them with the old 1-permit semaphore.

## Fix
Pass the resolved `(session_mode, cap)` tuple from a single manifest read into `agent_concurrency_for`, or bump a cache epoch on `replace_manifest` so the cap is reread.

**Minimum signal** (even if the cache is intentionally retained per CLAUDE.md's "kill the agent to re-read" policy): emit a `WARN` from `manifest_swap` whenever `session_mode` or `max_concurrent_invocations` changed, telling operators a respawn is required to take effect. The current `tracing::warn!` at `accessors.rs:967-976` fires only on first resolution — never on subsequent hot-reload. Better: when an agent has zero outstanding permits, invalidate the cached semaphore so the new cap takes effect immediately (avoids the permit-loss race).

## Tests
- Set agent to `Persistent + cap=1`, send 10 messages, observe cap holds. Hot-reload to `New + cap=5`, send 10 messages, observe new cap.
- Assert `manifest_swap` emits exactly one `WARN` per agent when `max_concurrent_invocations` changes.
