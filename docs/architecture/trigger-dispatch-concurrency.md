# Trigger dispatch concurrency

How event triggers (`TaskPosted`, `MessageReceived`, …) fan out to agents
under bounded concurrency. Scope is **the trigger dispatcher only** —
`agent_send`, channel bridges, and cron all serialize on the existing
`agent_msg_locks` / `session_msg_locks` inside `send_message_full` and
are not throttled by the per-agent cap.

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

**Tuning note.** Both permits are held for the full agent-loop duration
(seconds for short replies, minutes for tool-heavy turns). If a slow
trigger fire occupies one of the 8 default lane permits, fast triggers
queue behind it. Operators with mixed fast/slow trigger workloads should
either (a) raise `trigger_lane` proportionally to expected slow-fire
ratio, or (b) split the slow workload onto a separate agent so its
per-agent cap absorbs the contention instead of the global lane.

## Resolution order — per-agent cap

```
manifest.max_concurrent_invocations         (Some(n) wins)
    │
    └─ None ─► queue.concurrency.default_per_agent
                   │
                   └─ rewritten to 1 if 0 (validation)
```

`max(1)` floor is enforced at resolution time so a manifest typo of
`max_concurrent_invocations = 0` is silently treated as `1` (a 0-permit
semaphore would deadlock the agent). The same floor applies to
`default_per_agent` after `validation.rs` rewrites a TOML `0` to `1`.

## Persistent + cap > 1 is auto-clamped

Concurrent invocations against a single persistent session would race on
the message-history append. The resolver auto-clamps this combination
to 1 (with a `WARN` log) instead of refusing to start — operators see
the warning at first dispatch rather than discovering a hard error
deep into a config rollout:

```
WARN max_concurrent_invocations > 1 ignored — session_mode = "persistent"
     cannot run parallel invocations safely; clamped to 1.
     Set session_mode = "new" on the manifest to enable parallel fires.
```

The clamp is gated on the **manifest** `session_mode` only. A
per-trigger `session_mode_override = "new"` does NOT unlock the cap:
the per-agent semaphore is sized once on first dispatch from the
manifest default, and the dispatcher does not rescan triggers when
acquiring the per-agent permit. To run a `New`-mode workload in
parallel, set `session_mode = "new"` on the manifest itself; per-trigger
overrides are useful for one-off `New` fires but cannot escape an
already-clamped semaphore.

To run an agent in parallel, set:

```toml
[agents.<role>]
session_mode = "new"
max_concurrent_invocations = 4
```

### Gotcha: per-trigger `session_mode = "new"` does NOT grant parallelism

This is the most common surprise, so it is spelled out in full here.

A per-trigger override (`[[triggers]] … session_mode = "new"`) controls
**session isolation** — whether *that* fire gets a fresh `SessionId` — but
it does **not** resize the per-agent concurrency semaphore. Those are two
independent gates:

- **Fresh session per fire** comes from the *effective* session mode:
  `trigger.session_mode` (the per-trigger override) `>` manifest
  `session_mode`. Resolved at dispatch in
  `triggers_and_workflow.rs` (the `effective_mode` branch): `New` →
  `Some(SessionId::for_trigger_fire(agent, trigger_id, fire_time))` (a
  deterministic v5 UUID since #5604), `Persistent` → `None`.
- **Per-agent parallelism cap** comes from `agent_concurrency_for`, which
  reads the **manifest only** — never the per-trigger override. It is
  sized **once**, lazily, on the agent's first dispatch, from a single
  manifest snapshot of `(session_mode, max_concurrent_invocations)`.

So a trigger that mints fresh sessions still queues behind a 1-permit
per-agent semaphore whenever the **manifest** default is `Persistent`
(the default) — because the `Persistent + cap > 1 → 1` clamp already
fired when the semaphore was created. The override mints isolated
sessions that then run *serially*. The dispatcher never rescans triggers
when acquiring the per-agent permit, so a later override can't grow an
already-sized semaphore.

**Effective per-agent cap — exact resolution order.** Override is absent
from this list on purpose: it cannot enter the cap computation.

```
manifest.session_mode == Persistent  AND  max_concurrent_invocations > 1
    └─► clamped to 1   (WARN logged; per-trigger override is IGNORED here)

manifest.max_concurrent_invocations = Some(n)   (any other session_mode)
    └─► n

manifest.max_concurrent_invocations = None
    └─► queue.concurrency.default_per_agent   (default 1)

…then max(1) floor applied to all branches.
```

**Surprising config (mints fresh sessions, still serial).** The manifest
default stays `Persistent`, so the cap clamps to 1 regardless of the
per-trigger `New`:

```toml
[agents.researcher]
# session_mode defaults to "persistent"
max_concurrent_invocations = 4   # clamped to 1 — WARN at first dispatch

[[agents.researcher.triggers]]
event = "task_posted"
session_mode = "new"             # isolates each fire, but does NOT lift the cap
```

**Correct config for real per-trigger parallelism.** Move `New` to the
**manifest default** so the clamp does not fire and the requested cap
survives; keep the per-trigger override only where you want a one-off
deviation:

```toml
[agents.researcher]
session_mode = "new"             # manifest default — clamp does not fire
max_concurrent_invocations = 4   # honored: up to 4 fires run in parallel

[[agents.researcher.triggers]]
event = "task_posted"
# inherits manifest "new"; no per-trigger override needed for parallelism
```

If you genuinely want most fires `Persistent` but one trigger parallel,
that is **not** expressible today: the per-agent cap is a manifest-wide
property, and a `Persistent` manifest is hard-capped at 1. Split that
workload onto a separate agent whose manifest is `New`.

This is intentional, not a bug: a single `Persistent` session has one
shared message history, and concurrent appends to it are undefined
(see *Persistent + cap > 1 is auto-clamped* above). The override is a
session-isolation knob, not a concurrency knob.

## What honors `session_mode = "new"` for parallelism

| Path | Materializes session_id? | Per-agent cap applies? | Effect on locks |
|---|---|---|---|
| Event trigger dispatch (this doc) | yes — `SessionId::for_trigger_fire` per fire (deterministic v5 UUID since #5604) | **yes** | per-session mutex; agent cap throttles parallelism |
| Cron with `job.session_mode = New` | yes — `SessionId::for_cron_run(agent, run_key)` | no | deterministic session id; serializes via per-session mutex |
| `agent_send` | yes (when receiver manifest = New) | no | per-session mutex; not throttled by per-agent cap — see scope note above |
| Channel messages (Telegram, Slack, …) | no — always `SessionId::for_channel(agent, ch:chat)` | no | per-channel session; serial per chat |
| Forks | no — forced `Persistent` to preserve prompt cache | no | per-agent serialization |

## Lifecycle

- The per-agent semaphore is created lazily on first dispatch.
- It is removed by `gc_sweep` when the agent leaves the registry —
  i.e. on `agent kill` / despawn, **not** on a status flip.
- It is **not** invalidated on `manifest_swap` hot-reload. To pick up
  a new `max_concurrent_invocations` operators must:
  1. Kill the agent (`agent kill <id>` / API equivalent) and let it
     respawn from the updated manifest, or
  2. Restart the daemon.

  An in-place "reactivate" / status flip that keeps the agent in the
  registry will silently retain the old capacity. The lazy-init design
  trades hot-reload immediacy for the simplicity of avoiding a
  permit-loss race during live config reloads.

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
