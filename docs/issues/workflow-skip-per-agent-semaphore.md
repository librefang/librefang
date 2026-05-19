# Workflow path unconditionally skips the per-agent semaphore, allowing `max_concurrent_invocations` to be bypassed

**Severity:** Medium
**Category:** Kernel orchestration logic
**Labels:** `concurrency`, `medium`

## Affected files
- `crates/librefang-kernel/src/kernel/triggers_and_workflow.rs:334-336`

## Description

The comment at `:335` says "the workflow path needs no per-agent semaphore or session." But workflow steps eventually call `send_message_full` (same file, `:738+`), which issues per-agent LLM calls.

Consequences:

- The trigger-dispatch stage drops the per-agent semaphore;
- The per-session lock only kicks in inside `send_message_full` for `New` mode;
- A workflow fanning out N parallel `Persistent` steps to the **same** agent can exceed `max_concurrent_invocations`, until the downstream messaging-layer per-agent mutex serializes them — but a per-agent mutex is not a numeric cap; that is the semaphore's job.

## Recommendation

Plumb the per-agent semaphore into the workflow dispatcher, keyed on the step's target agent. If this is intentionally exempt, then:

1. Document it explicitly in `docs/architecture/trigger-dispatch-concurrency.md`;
2. When a workflow step's target agent has a non-default `max_concurrent_invocations`, emit a `WARN` at trigger-registration time.
