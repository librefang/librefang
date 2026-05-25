# Rate-limit owner notification

## Why

Before this feature, when the underlying LLM provider rate-limited the agent
kernel-wide and retries were exhausted, the daemon logged a one-line warning
and dropped the turn:

```
WARN ... Claude Code CLI streaming subprocess exited with error
       exit_code=1 stderr=
```

The user who triggered the turn (an owner chatting with their agent on
WhatsApp / Telegram / etc.) saw **nothing**: no acknowledgement, no error
banner, no estimate of when the agent would come back. The original
incident (2026-05-20, chat `!CbDmKayZoOd53YJsAOAJ`, msg `355199`) was an
owner sending an image to Ambrogio while the shared OAuth-Max account was
inside its rolling 5-hour cap; the agent stayed silent for forty-five
minutes before the quota rolled over.

This feature closes that gap: when retries are exhausted on a
`RateLimited` error and the request originated from a channel, the agent
loop renders a concise, operator-configurable, timezone-aware notification
template and dispatches it through the same channel the request arrived
on. The original error still propagates upward — the notify is a
side-effect on the error path, not a substitute for it.

## Three layers

### 1. Driver — capture `rate_limit_event`

`crates/librefang-llm-drivers/src/drivers/claude_code.rs`

The Claude CLI's `--output-format stream-json` mode emits a
`rate_limit_event` line on every streaming call:

```json
{
  "type": "rate_limit_event",
  "rate_limit_info": {
    "status": "allowed",
    "resetsAt": 1779282600,
    "rateLimitType": "five_hour"
  }
}
```

The driver captures this into a `last_rate_limit_info: Option<RateLimitInfo>`
that persists across the stream loop. On non-success CLI exit, if a
`resets_at` is available we construct an `LlmError::RateLimited` whose
`message` payload embeds machine-readable `resets_at_unix=<ts>` and
`rate_limit_type=<kind>` markers alongside the human-readable text. The
existing text-pattern detector (`detect_cli_error_in_text`) is preserved
as the fallback path; its rate-limit branch is enriched with the same
header shape so the downstream parser sees a uniform format.

### 2. Config — `[system]` and `[rate_limit_notify]`

`crates/librefang-types/src/config/types.rs` adds two new structs:

```toml
# config.toml
[system]
timezone = "Europe/Rome"

[rate_limit_notify]
enabled  = true
template = "⏸️ Limite Claude raggiunto. Reset alle {reset_time}. Ti rispondo dopo."
```

`AgentManifest.rate_limit_notify` (in `agent.toml`) provides the per-agent
override:

```toml
[rate_limit_notify]
enabled  = true
template = "Signore, il maggiordomo è in permesso fino alle {reset_time} ({reset_in_minutes} min). 🎩"
```

Resolution order, walked by `librefang_runtime::rate_limit_notify::resolve_config`:

1. **Per-agent** (`AgentManifest.rate_limit_notify`) — wins when its
   `enabled = true` OR its `template = Some(...)`.
2. **Kernel-global** (`KernelConfig.rate_limit_notify`) — used otherwise.
3. **Hardcoded fallback** (`DEFAULT_TEMPLATE`) — when neither layer
   supplies a template.

`enabled = false` anywhere in the resolved config short-circuits the
dispatch.

### 3. Runtime — dispatch from the agent loop

`crates/librefang-runtime/src/rate_limit_notify.rs` exposes
`dispatch_via_kernel(...)` which the agent loop calls on the `Err(e)`
path of both `call_with_retry` (`agent_loop/mod.rs`) and
`stream_with_retry` (`agent_loop/run_streaming.rs`). The helper:

1. Parses the error string for the `[rate_limit_defer_ms]` marker that
   `agent_loop/retry.rs::handle_retryable_llm_error` appends on
   retry-budget exhaustion. Errors without that marker are not
   rate-limit failures and the helper returns immediately.
2. Pulls the active `RateLimitNotifyConfig` and operator timezone from
   the `KernelHandle` via two trait getters added in
   `crates/librefang-kernel-handle/src/lib.rs`:
   `rate_limit_notify_config()` and `system_timezone()`.
3. Recovers the precise `reset_at` unix-seconds from the embedded
   `resets_at_unix=<ts>` marker (Layer 1). Falls back to
   `now + retry_after_ms` when the marker is missing (e.g. non-Claude
   providers that haven't been migrated yet).
4. Consults the dedup LRU (`should_dispatch`) — see below.
5. Renders the template with `render_rate_limit_template`.
6. Calls `ChannelSender::send_channel_message` directly through the
   `KernelHandle` (which extends `ChannelSender`). The notify
   **bypasses** the agent loop because re-entering it would just hit
   the same exhausted quota.

## Template placeholders

Rendered by a simple `{name}` substitution (no Tera/Handlebars):

| Placeholder            | Example                            |
|------------------------|------------------------------------|
| `{reset_time}`         | `13:40`                            |
| `{reset_time_full}`    | `2026-05-20 13:40:00 Europe/Rome`  |
| `{reset_in_minutes}`   | `45`                               |
| `{reset_tz}`           | `Europe/Rome` (or `UTC` on fallback)|
| `{agent_name}`         | `ambrogio`                         |

**Unknown placeholders are kept verbatim** (`{bogus}` stays `{bogus}` in
the delivered message). That's deliberate: a typo in an operator-supplied
template should surface visibly in the chat, not be silently elided so
the operator never notices.

## Timezone resolution

`resolve_timezone_str` parses `[system].timezone` as an IANA name through
`chrono_tz::Tz::from_str`. The string can be:

- **None / empty** → defaults to `Tz::UTC`.
- **Unparseable** (e.g. `"Not/Real"`) → falls back to `Tz::UTC` and logs
  a single `warn!` per process (gated by `TIMEZONE_WARN_ONCE`) so the
  operator sees the misconfiguration in the daemon log without it
  spamming on every turn.
- **Valid** (e.g. `"Europe/Rome"`) → used as-is. DST transitions are
  handled by `chrono-tz`'s `LocalResult`.

## Dedup window

`should_dispatch(agent_id, peer, reset_at_unix)` consults a process-local
LRU (`OWNER_NOTIFY_DEDUP`, capacity 64) keyed by
`(agent_id, peer, reset_bucket_5min)` where the bucket is computed as
`reset_at_unix.div_euclid(300)`.

Why bucket the **reset** time and not the **current** time: a rate-limit
incident produces multiple retries from the same peer landing on the
same exhausted quota window. Bucketing by `now` would let a slow retry
storm straddle a wall-clock 5-minute boundary and notify twice;
bucketing by the reset timestamp guarantees exactly one notification per
quota window.

The LRU evicts the oldest entry at saturation rather than dropping new
notifications, so even past 64 distinct `(agent, peer, window)`
combinations the worst case is a duplicate ping after the original
entry rolls off — strictly better than the silent-failure baseline this
feature replaces.

## Operator examples

### Enable for every agent

```toml
# ~/.librefang/config.toml
[system]
timezone = "Europe/Rome"

[rate_limit_notify]
enabled = true
template = "⏸️ Limite Claude raggiunto. Reset alle {reset_time} ({reset_in_minutes} min). Ti rispondo dopo."
```

### Per-agent override

```toml
# ~/.librefang/agents/ambrogio/agent.toml
[rate_limit_notify]
enabled  = true
template = "Signore, il maggiordomo è in permesso fino alle {reset_time} ({reset_in_minutes} min). 🎩"
```

### Disable for one noisy agent while keeping the global default on

```toml
# ~/.librefang/agents/cron-worker/agent.toml
[rate_limit_notify]
enabled  = false
# template intentionally absent — explicit `enabled = false` overrides the kernel default
```

## What this does NOT do

- **Does not retry the failed turn.** The original error propagates
  upward unchanged; the channel bridge's existing
  `RATE_LIMIT_DEFER_MARKER` path still drives journal-status deferral
  and re-dispatch on a ticker.
- **Does not catch every provider's rate-limit shape.** Layer 1
  (Claude CLI) is the wired path. Other drivers that surface
  `LlmError::RateLimited` get the fallback experience: the notify
  fires with a `now + retry_after_ms` reset estimate instead of a
  precise wall-clock from the provider.
- **Does not notify on non-channel callers** (cron, autonomous loops,
  direct HTTP API). The `sender_channel` / `sender_id` metadata that
  the channel bridges stamp on the manifest is required.
- **Does not invalidate the dedup LRU on hot-reload.** Once we've
  told the owner about a reset, we don't want to spam them again
  when a config edit causes a manifest reload. The LRU clears on
  daemon restart.
