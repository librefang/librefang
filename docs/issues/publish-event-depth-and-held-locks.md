# Trigger spawn re-establishes `PUBLISH_EVENT_DEPTH` but misses a `held_agent_locks` contract test

**Severity:** Low
**Category:** Concurrency, races, deadlocks
**Labels:** `concurrency`, `test-coverage`, `low`

## Affected files
- `crates/librefang-kernel/src/kernel/triggers_and_workflow.rs:393-395, 510-530`
- Context: `crates/librefang-runtime/src/held_agent_locks.rs`, `crates/librefang-kernel/src/kernel/messaging.rs:785`

## Description

The trigger spawn re-establishes only the `PUBLISH_EVENT_DEPTH` task-local; it **does not** re-establish `held_agent_locks` — this is intentional, because a spawned task does not hold the parent's locks. `send_message_full` re-wraps it on entry via `held_agent_locks::scope` (`messaging.rs:785`).

The risk: if anyone in the future introduces a path that takes a lock inside a spawn and then calls `send_message_full` directly (without first opening a scope), the #5125 "A→B→A self-call rejection" silently breaks.

## Recommendation

Add an integration test: trigger an A→B→A trigger chain and assert that the #5125 "reentrant agent_send rejected" message fires. Add a comment at `:394` stating explicitly that "spawned tasks must go through `send_message_full_with_upstream` to rebuild the scope."
