# [Low] Concurrency Low — MQTT task untracked, process-manager readers detached, `held_locks` `RefCell`

**Severity:** Low · **Domain:** Concurrency
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | The MQTT eventloop inner spawn is not registered with `BridgeManager::track_adapter_task` → `abort()` will not actually kill it | `channels/src/mqtt.rs:292` vs `bridge.rs:1846-1862` |
| reader detach | Process-manager reader tasks are silently detached — spawned but never joined; cleanup race | process-manager implementation |
| RefCell across await | `held_agent_locks` uses `RefCell` in an async context — borrows across `await` are fragile, easy to panic | `runtime/src/held_agent_locks.rs` |

## Combined fix plan

1. (this) Extend the `track_adapter_task` API so adapters can register inner `JoinHandle`s; the MQTT eventloop spawn goes through the helper.
2. (reader detach) Move process-manager readers into a `JoinSet` / register them with the supervisor; on cleanup, `join_all` rather than detach.
3. (RefCell across await) Replace `RefCell` with `tokio::sync::Mutex` or a `Cell<bool>` + custom guard; or ensure no borrow scope spans an `await`.
