# Trigger `New`-mode SessionId is a random UUID; logs cannot correlate fires to sessions

**Severity:** Medium
**Category:** Kernel orchestration logic
**Labels:** `observability`, `diagnostics`, `medium`

## Affected files
- `crates/librefang-kernel/src/kernel/triggers_and_workflow.rs:344-347`
- Reference: cron uses `SessionId::for_cron_run(agent, "<job_id>:<rfc3339>")` (reproducible)

## Description

In `SessionMode::New`, the dispatcher builds `SessionId::new()` — a **random v4 UUID**. An operator reading the log "trigger X fired at T" cannot map back to a specific SessionId. Two same-instant fires with a shared event id make it impossible for the diagnostics tooling to reconstruct "which fire corresponds to which session."

## Recommendation

Add the dual function:

```rust
impl SessionId {
    pub fn for_trigger_fire(agent: &AgentId, trigger_id: &TriggerId, fire_time: DateTime<Utc>) -> Self {
        // similar to CRON_RUN_SESSION_NAMESPACE, UUID v5
    }
}
```

Pin a contract test modeled on the cron `fire_session_override_new_matches_for_cron_run_contract_3657` test.
