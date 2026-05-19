# [Low] Concurrency Low — process-manager readers detached, `held_locks` `RefCell`

**Severity:** Low · **Domain:** Concurrency
**Status:** The MQTT half of the original rollup is moot — the `channel-mqtt` adapter was deleted (see CHANGELOG `[Unreleased]` ### Removed). Remaining sub-findings stand.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| reader detach | Process-manager reader tasks are silently detached — spawned but never joined; cleanup race | process-manager implementation |
| RefCell across await | `held_agent_locks` uses `RefCell` in an async context — borrows across `await` are fragile, easy to panic | `runtime/src/held_agent_locks.rs` |

## Combined fix plan

1. (reader detach) Move process-manager readers into a `JoinSet` / register them with the supervisor; on cleanup, `join_all` rather than detach.
2. (RefCell across await) Replace `RefCell` with `tokio::sync::Mutex` or a `Cell<bool>` + custom guard; or ensure no borrow scope spans an `await`.
