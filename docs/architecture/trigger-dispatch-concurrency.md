# Trigger dispatch concurrency

How event triggers (`TaskPosted`, `MessageReceived`, …) and `agent_send`
fan out to agents under bounded concurrency.

## The three layered caps

A trigger match goes through three independent gates, in order:

```
trigger fires
     │
     ▼
┌──────────────────────────────────────────────────────────────┐
│ 1. Global lane semaphore  (Lane::Trigger)                    │
│    config: queue.concurrency.trigger_lane  (default 8)       │
│    Caps total in-flight trigger dispatches kernel-wide so a  │
│    runaway producer (50× task_post in a tight loop) cannot   │
│    spawn unbounded tokio tasks racing for everyone else's    │
│    mutexes.                                                  │
└──────────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────────┐
│ 2. Per-agent semaphore  (one per agent_id)                   │
│    manifest: max_concurrent_invocations                      │
│    fallback: queue.concurrency.default_per_agent (default 1) │
│    Caps how many of THIS agent's fires run in parallel.      │
└──────────────────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────────────────────┐
│ 3. Per-session mutex  (existing session_msg_locks)           │
│    Reached only when the dispatcher materialized a fresh     │
│    SessionId for `session_mode = "new"` fires. Persistent    │
│    fires fall back to the per-agent mutex inside             │
│    send_message_full and serialize there.                    │
└──────────────────────────────────────────────────────────────┘
```

All three permits are acquired before the agent loop starts and dropped
on task exit. The lane permit is acquired with `acquire_owned()` so it
moves into the spawned `tokio::spawn` task and releases on completion
regardless of success/failure.

## Resolution order — per-agent cap

```
manifest.max_concurrent_invocations         (Some(n) wins)
    │
    └─ None ─► queue.concurrency.default_per_agent
                   │
                   └─ rewritten to 1 if 0 (validation)
```

`max(1)` floor is enforced at resolution time — `0` would deadlock the
agent.

## Persistent + cap > 1 is auto-clamped

Concurrent invocations against a single persistent session would race on
the message-history append. The resolver detects this combination and
clamps the cap to 1 with a `WARN` log:

```
WARN max_concurrent_invocations > 1 ignored — session_mode = "persistent"
     cannot run parallel invocations safely; clamped to 1.
     Set session_mode = "new" on the manifest (or per-trigger) to enable
     parallel fires.
```

To run an agent in parallel, set:

```toml
[agents.<role>]
session_mode = "new"
max_concurrent_invocations = 4
```

## What honors `session_mode = "new"` for parallelism

| Path | Materializes session_id? | Effect on per-agent lock |
|---|---|---|
| Event trigger dispatch (this doc) | yes — `SessionId::new()` per fire | bypassed; per-session mutex applies |
| Cron with `job.session_mode = New` | yes — `SessionId::for_cron_run(agent, run_key)` | bypassed; deterministic session id |
| `agent_send` | yes (when receiver manifest = New) | bypassed |
| Channel messages (Telegram, Slack, …) | no — always `SessionId::for_channel(agent, ch:chat)` | per-channel session, bypassed at the lock level but not parallel per chat |
| Forks | no — forced `Persistent` to preserve prompt cache | per-agent serialization |

## Lifecycle

- The per-agent semaphore is created lazily on first dispatch.
- It is removed by the periodic registry GC pass when the agent is
  killed/removed.
- It is **not** invalidated on manifest hot-reload. Operators changing
  `max_concurrent_invocations` should reactivate the agent (or the
  hand) for the new capacity to take effect — this avoids a permit-loss
  race during live config reloads.

## Observability

`GET /api/queue/status`:

```json
{
  "lanes": [
    {"lane": "main",     "active": 0, "capacity": 3},
    {"lane": "cron",     "active": 0, "capacity": 2},
    {"lane": "subagent", "active": 0, "capacity": 3},
    {"lane": "trigger",  "active": 0, "capacity": 8}
  ],
  "config": {
    "max_depth_per_agent": 0,
    "max_depth_global": 0,
    "task_ttl_secs": 3600,
    "concurrency": {
      "main_lane": 3,
      "cron_lane": 2,
      "subagent_lane": 3,
      "trigger_lane": 8,
      "default_per_agent": 1
    }
  }
}
```

The dashboard's runtime page renders all four lanes from this endpoint
and surfaces the queue config block including the new
`default_per_agent` field. Per-agent cap values are read from the
agent manifest.

## Why not reuse `Lane::Main`?

`main_lane` is documented as "user messages" — operators tune it for
chat-bridge throughput. Routing trigger fires through it would silently
re-bind that knob. A new `Lane::Trigger` keeps existing operators'
mental models intact and gives trigger throughput its own independent
budget.

## Related code

- `crates/librefang-types/src/agent.rs` — `AgentManifest.max_concurrent_invocations`
- `crates/librefang-types/src/config/types.rs` — `QueueConcurrencyConfig`
- `crates/librefang-runtime/src/command_lane.rs` — `Lane::Trigger`, `CommandQueue`
- `crates/librefang-kernel/src/kernel/mod.rs` — `agent_concurrency_for`, dispatch loop
