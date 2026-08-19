# `session_mode` resolution

`session_mode` decides whether an automated invocation reuses the agent's persistent session or starts a fresh one.
It is declared in the agent manifest (`agent.toml`), **not** in `config.toml`.

```toml
# ~/.librefang/workspaces/agents/<name>/agent.toml
session_mode = "persistent"   # default; "new" starts a fresh session per invocation
```

Session resolution lives in `execute_llm_agent` (`crates/librefang-kernel/src/kernel/agent_execution.rs`).

## Resolution order

Per-trigger override (`Trigger.session_mode: Option<SessionMode>`) or per-cron override (`CronJob.session_mode: Option<SessionMode>`) wins over the agent manifest default.
With neither set, the historical behaviour is `Persistent`.

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
