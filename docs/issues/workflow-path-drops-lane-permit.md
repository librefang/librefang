# Workflow path drops the `Lane::Trigger` permit too early; concurrent executions are unbounded

**Severity:** Medium
**Category:** Concurrency, races, deadlocks
**Labels:** `concurrency`, `dos`, `medium`

## Affected files
- `crates/librefang-kernel/src/kernel/triggers_and_workflow.rs:454-495`

## Description

The comment at `:454-459` admits the design: the `Lane::Trigger` permit is held only during workflow **resolution**; afterwards `tokio::spawn` runs `kernel.run_workflow(...)` — inside the spawn there is **no lane limiting**.

CLAUDE.md states the trigger lane is a "kernel-wide cap controlling total in-flight trigger fires," but a workflow-typed trigger is counted only during resolution; once it's in the spawn, it escapes. N bursty workflow triggers → N concurrent workflow runs.

`fire_timeout` prevents an infinite loop, but the lane invariant is already broken.

## Recommendation

Pick one:

1. Move `_lane_permit` into the spawned future, holding it throughout the whole workflow run;
2. Introduce a separate `Lane::Workflow` semaphore so workflow concurrency and trigger-dispatch concurrency do not affect each other.

Update `docs/architecture/trigger-dispatch-concurrency.md` accordingly.
