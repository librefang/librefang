# Sidecar channel adapters

LibreFang is **sidecar-first** for channels. A channel adapter is an
out-of-process subprocess in any language that speaks newline-delimited
JSON-RPC over stdin/stdout; the daemon supervises it. New channels are
written this way — the ~46 in-process Rust adapters that predate the
policy are grandfathered and frozen (see "Policy gate" below).

Why: a channel adapter is high-churn, low-risk integration glue. As an
in-process Rust module a bad adapter could panic the daemon, leak, or
drag a supply-chain dependency into the kernel process, and every new
one raised the contributor bar to "writes Rust + passes clippy +
rebuilds the workspace". As a supervised subprocess it is isolated
(a crash is not a daemon crash), restartable, and writable in ~40
lines of Python against a documented protocol.

This was delivered across the #5219 (protocol + supervision + config),
#5220 (Python SDK), #5221 (policy gate), and #5224 (ntfy migration)
series.

## Process model

```
 daemon (librefang-channels)                external subprocess
 ┌───────────────────────────┐              ┌──────────────────────┐
 │ SidecarAdapter             │   stdin     │ adapter (py/any lang) │
 │  supervisor task ──────────┼── cmds ────▶│  reads commands       │
 │   spawn_once()             │             │  talks to platform    │
 │   ChannelMessage stream ◀──┼── events ───┤  writes events        │
 │  (survives child restarts) │   stdout    │                       │
 └───────────────────────────┘   stderr ───▶ daemon log             │
                                              └──────────────────────┘
```

`SidecarAdapter` (`crates/librefang-channels/src/sidecar.rs`)
implements the same `ChannelAdapter` trait every in-process adapter
does, so the bridge, router, and approval paths treat it identically.
`start()` returns one long-lived `ChannelMessage` stream; the
supervisor re-spawns the child underneath it on crash without breaking
that stream.

## Protocol

Events (subprocess → daemon, stdout):

| method   | payload |
|----------|---------|
| `ready`  | `params`: `capabilities[]`, `account_id?`, `suppress_error_responses`, `notification_recipients[]`, `header_rules[]`, `protocol_version?` — all optional; bare `{"method":"ready"}` still parses |
| `message`| full `ChannelContent` (all 24 variants) + `is_group`, `thread_id`, sender, group roster, metadata |
| `typing` | `user_id`, `user_name`, `is_typing` |
| `error`  | `message` |

Commands (daemon → subprocess, stdin): `send`, `ready_ack`, `typing`,
`reaction`, `interactive`, `stream_start` / `stream_delta` /
`stream_end`, `heartbeat`, `shutdown`. Unknown methods (either
direction) are tolerated, not fatal — that is what lets a new daemon
send `ready_ack` to an older adapter and vice versa.

stdout carries only protocol frames. All adapter logging goes to
stderr (the SDK enforces this).

## Capability negotiation

An adapter declares what it supports in the `ready` event's
`capabilities`: `typing`, `reaction`, `interactive`, `thread`,
`streaming`, `typing_events`. Each gates the matching optional
`ChannelAdapter` method; an absent capability degrades to exactly the
pre-sidecar behaviour (plain text). `create_webhook_routes` stays
`None` for sidecars — an `axum::Router` can't cross stdio; an adapter
that needs inbound HTTP runs its own listener and POSTs events back
through stdout.

## Supervision

The supervisor owns the (re)spawn loop. State machine:

```
        ┌────────────────────────────────────────────┐
        ▼                                             │
   spawn_once ──▶ wait ready (≤ ready_timeout_secs) ──▶ running
        ▲              │ timeout                       │ child exits
        │ backoff      ▼                               ▼
        └──────── attempt++ ◀── ChildClosed ◀──────────┘
                   │
       attempt ≥ restart_max_retries ──▶ circuit-break (stop, one error log)
       clean Shutdown / receiver gone ──▶ stop (no restart)
       stable uptime ≥ reset_after   ──▶ attempt = 0
```

Backoff is exponential with dependency-free wall-clock jitter (≤20%),
capped at `restart_max_backoff_ms`. After `restart_max_retries`
consecutive failures the supervisor gives up with a single `error!`
(no crash-loop log spam). Backoff sleeps are shutdown-interruptible.
Backpressure: the inbound stream is a bounded `mpsc(message_buffer)`;
`overflow = "block"` (default — applies backpressure, never drops a
user message) or `"drop_newest"` (shed load for high-volume
notification adapters).

All tunables are per-adapter `[[sidecar_channels]]` config fields
(`restart`, `restart_initial_backoff_ms`, `restart_max_backoff_ms`,
`restart_max_retries`, `restart_reset_after_secs`,
`ready_timeout_secs`, `shutdown_grace_secs`, `message_buffer`,
`overflow`). `librefang.toml.example` documents them with defaults.

## Responsibility split

- **Process restart is the daemon's job.** The supervisor respawns a
  crashed child with backoff + circuit-break. An adapter must be
  *crash-safe*: hold no irreplaceable in-process state and re-announce
  `ready` on every fresh start (the SDK does this automatically).
- **Platform reconnect is the adapter's job.** Reconnecting a dropped
  Telegram long-poll / WebSocket / SSE stream is the adapter's
  transport concern (`librefang.sidecar.with_backoff` helps). It is
  independent of the daemon-managed process lifecycle.

## Policy gate

`crates/librefang-channels/src/channels-allowlist.txt` grandfathers the
in-process adapters that predate sidecar-first. The list only ever
**shrinks**: migrating an adapter to a sidecar and deleting its module
removes its line, after which it can never return in-process.

`scripts/hooks/pre-commit` (fast feedback) and `cargo xtask
channel-policy` — run unconditionally in the CI `quality` job, the
authoritative gate — reject any file under
`crates/librefang-channels/src/{<name>.rs, <name>/*.rs}` containing
`ChannelAdapter for` whose basename is not allowlisted. Known accepted
limitation: a macro-generated impl, or an adapter impl added inside an
already-allowlisted file, is not detected — this is a policy ratchet,
not a security boundary.

## Worked example: ntfy

`examples/sidecar-channel-python/ntfy_adapter.py` is the canonical
migration (#5224). It replaced the former in-process
`librefang-channels::ntfy` adapter with behaviour preserved (SSE
subscribe, `/command` parsing, `title`→sender, `topic` metadata,
chunked plain-text publish, optional Bearer auth, backoff reconnect).
`NtfyConfig` / `[channels.ntfy]` were removed and `ntfy` deleted from
the allowlist, so the gate now permanently blocks an in-process ntfy.
This was a **breaking config change**: an existing `[channels.ntfy]`
block is re-declared as a `[[sidecar_channels]]` running
`ntfy_adapter.py`. The separate ntfy *push-notification provider*
(`push_provider = "ntfy"`) is an unrelated feature and was untouched.

## Long-tail migration backlog

ntfy proved the pipeline but also showed that fully removing one
in-process channel's config type has a wide, kernel-touching,
**breaking** blast radius (config schema, api routes/features, kernel
`channel_sender` registry, cli TUI, validation, golden). Remaining
candidates (genuinely text-only / low-traffic, e.g. `gitter`,
`gotify`) should migrate **opportunistically** — when someone next
touches one, or on explicit maintainer decision — not as a batch
rewrite. New channels are sidecar by policy, so the in-process set
only shrinks over time without a forced campaign. Each migration is a
breaking change for that channel's `[channels.<x>]` config and must be
called out in `CHANGELOG.md` under `### Changed`.
