# `TriggerEngine::register` has no per-agent cap; manifest reconcile does not clamp either

**Severity:** High
**Category:** DoS / resource exhaustion
**Labels:** `dos`, `manifest`, `high`

## Affected files
- `crates/librefang-kernel/src/triggers.rs:383-479` (`register_with_target_enabled`)
- `crates/librefang-kernel/src/triggers.rs:908-999` (`reconcile_manifest_triggers`)
- Reference: `crates/librefang-kernel/src/cron.rs:81` (`max_total_jobs`) + `crates/librefang-types/src/scheduler.rs:507` (`MAX_JOBS_PER_AGENT=50`)

## Description

`register_with_target_enabled` (`:447`) unconditionally performs `self.triggers.insert(id, trigger)` + `agent_triggers.entry(agent_id).or_default().push(id)` — with no length check.

`reconcile` walks the manifest's `triggers: Vec<ManifestTrigger>` and creates a runtime trigger for every undeclared entry.

Consequences:

- A malicious or buggy `agent.toml` with 100k triggers loads them all;
- Per-event match scanning is O(N);
- `DEFAULT_MAX_TRIGGERS_PER_EVENT=10` (`triggers.rs:26`) caps **fire** count but not **scan** cost;
- The persisted `triggers.json` grows in lockstep.

Cron has two layers of caps; triggers are asymmetric here.

## Recommendation

Add:

```rust
pub const MAX_TRIGGERS_PER_AGENT: usize = 50;
// config: queue.concurrency.max_total_triggers (default 500)
```

`register_with_target_enabled` and `reconcile_manifest_triggers` must check the cap before creating and return an explicit error when exceeded — **do not** silently discard. Operators need to know when `agent.toml` is being truncated.

Integration test: a manifest with 51 triggers makes reconcile error.
