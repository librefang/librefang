# `session_mode` resolution

`session_mode` decides whether an automated invocation reuses the agent's persistent session or starts a fresh one.
It is declared in the agent manifest (`agent.toml`), **not** in `config.toml`.

```toml
# ~/.librefang/workspaces/agents/<name>/agent.toml
session_mode = "persistent"   # default; "new" starts a fresh session per invocation
```

Session resolution lives in `execute_llm_agent` (`crates/librefang-kernel/src/kernel/agent_execution.rs`).

## Resolution order

An explicit caller-supplied session id (`session_id_override`) sits above the whole ladder: when present, `execute_llm_agent` uses that session and never consults `session_mode` at all.
It reaches the kernel from `MessageRequest.session_id` on `POST /api/agents/{id}/message`, from the WebSocket handler's pinned session, from `librefang message --session-id <UUID>` (issue #7605), and from the cron `New` branch described below.
The only validation is ownership: a session id belonging to a different agent is rejected with `InvalidInput`, so one agent's id can never read another's history.

Below that, a per-trigger override (`Trigger.session_mode: Option<SessionMode>`) or per-cron override (`CronJob.session_mode: Option<SessionMode>`) wins over the agent manifest default.
With neither set, the historical behaviour is `Persistent`.

## What an explicit session id means per mode

| Manifest `session_mode` | No explicit id | Explicit id |
|---|---|---|
| `Persistent` | The agent's canonical registry session — every caller shares one history. | The named session; the canonical session is untouched. |
| `New` | A freshly minted `SessionId::new()` per invocation, discarded after. | The named session, reused across invocations — continuity is restored for that conversation. |

The asymmetry is deliberate: `session_mode` answers "which session does an *unaddressed* invocation get", and an explicit id means the invocation is addressed.
A public multi-user front-end therefore does not need `session_mode = "new"` at all — it wants one durable session per end-user, which is exactly `Persistent` plus an explicit id.

## Which call paths honour it

| Path | Honours `session_mode` |
|---|---|
| Event triggers | yes |
| `agent_send` | yes |
| Cron jobs | yes, since #3597 / #3657 |
| Channel messages | no — always `SessionId::for_channel(agent, "<channel>:<chat>")` |
| Forks | no — forced `Persistent` to preserve the prompt cache |

## Cron in detail

The resolution helper is `cron::cron_fire_session_override` (`crates/librefang-kernel/src/cron.rs`).

Effective mode = per-job `CronJob.session_mode` > agent manifest `session_mode` > historical `Persistent`.

**`Persistent` (or unset).**
The cron tick synthesizes `SenderContext { channel: "cron" }` and `send_message_full` derives `SessionId::for_channel(agent, "cron")`.
All fires of all cron jobs for that agent therefore share one `(agent, "cron")` persistent session.
This is the historical behaviour and the one that reuses the provider prompt cache.

**`New`.**
`cron_fire_session_override` returns an explicit `SessionId::for_cron_run(agent, "<job_id>:<rfc3339_fire_time>")`, passed as `session_id_override` into `send_message_full`.
The override path wins over the channel-derived branch, so each fire lands on its own deterministic, isolated session.
Prior fires never leak into the current run, and the persistent `(agent, "cron")` session stays untouched.

## Choosing

When creating a trigger or a cron job, pick consciously:

- `Persistent` — continuity across fires, prompt-cache reuse, lower cost.
- `New` — isolation, fresh context per fire, no cross-fire contamination.

Related: [trigger dispatch concurrency](./trigger-dispatch-concurrency.md) (a persistent session plus a per-agent cap above 1 is auto-clamped to 1), and [cron session sizing](./cron-session-sizing.md).
