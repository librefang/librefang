# Sidecar channel adapters

LibreFang is **sidecar-first** for channels.
A channel adapter is an out-of-process subprocess in any language that speaks newline-delimited JSON-RPC over stdin/stdout; the daemon supervises it.
The in-process Rust adapters that predated the policy have all been migrated or removed — `crates/librefang-channels/src/channels-allowlist.txt` permits only the `sidecar` trampoline itself today (see "Policy gate" below).

Why: a channel adapter is high-churn, low-risk integration glue across ~28 platforms with independent dependency trees and shifting APIs.
As an in-process Rust module each one would panic the daemon on failure, drag its supply chain into the kernel's trust boundary, and require a full workspace rebuild + daemon restart to iterate on.
As a supervised subprocess each adapter is isolated (a crash is a `waitpid` event, not a daemon outage), its dependency tree is sealed away from the kernel, and the iteration loop is a subprocess restart.
The contributor-bar benefit — writable in ~40 lines against a documented protocol — is real but no longer load-bearing on its own; the architectural case that survives AI codegen is in *Why subprocess, not in-process Rust?* below.

This was delivered across the #5219 (protocol + supervision + config),
#5220 (Python SDK), #5221 (policy gate), and #5224 (ntfy migration)
series.

## Why subprocess, not in-process Rust?

AI codegen has narrowed the practical gap between writing Python and writing Rust, which weakens — but does not remove — the case for the sidecar boundary.
The contributor-ergonomics argument ("anyone can write a ~40-line Python adapter against a documented protocol") is real but no longer load-bearing on its own: a model that writes Python against this protocol can also write Rust against it.
So why keep the process boundary?

Three properties survive when the language gap closes:

1. **Crash isolation.**
   An in-process `ChannelAdapter` that panics, deadlocks, or trips an integer overflow ends the entire daemon process — every other adapter, every active agent session, every HTTP route.
   A sidecar crash is a `waitpid` event; the supervisor restarts it under exponential backoff with a circuit-breaker, and the rest of the daemon never notices.
   This property is independent of who wrote the adapter and in what language.

2. **Supply-chain confinement.**
   Each platform SDK (telegram bot client, discord gateway, slack socket-mode handshake, whatsapp business API, …) is its own dependency tree.
   In-process they would compose into the kernel binary's transitive dependency set, and every `cargo audit` finding in any one of ~28 platform SDKs would become a finding on the kernel itself.
   As subprocesses they are sealed: a vulnerability in the WhatsApp adapter's HTTP client cannot reach the memory layer, the LLM driver keys, or another channel.

3. **Iteration loop.**
   Channel APIs change with the platform.
   A sidecar adapter is one file to edit, then a subprocess restart — seconds.
   An in-process adapter requires `cargo build` against the full workspace and a daemon restart that drops every active agent session.
   The same edit at the same quality bar costs substantially more wall-clock time when it is in-tree, regardless of who wrote the code.

The wire protocol (see [`sidecar-protocol.md`](./sidecar-protocol.md)) is newline-delimited JSON over stdio; nothing in it is Python-specific.
Two first-party SDKs against the same conformance corpus (`conformance/sidecar/corpus/`) ship today:

- **Python** — `sdk/python/librefang/sidecar/`.
  The lowest-friction substrate for the ~28 in-process adapters that were migrated through the #5219 → #5459 series.
- **Rust** — `sdk/rust/librefang-sidecar/`.
  For adapters that need a stdlib-shaped binary, want type-safe access to the inbound command set without going through `serde_json::Value` by hand, or want to reuse a Rust transport crate that an external ecosystem has already hardened.
  Inherits every architectural property above without paying the Python interpreter's startup or memory cost.
  See [`rust-sidecar-sdk.md`](./rust-sidecar-sdk.md) for the SDK reference and [`rust-telegram-sidecar.md`](./rust-telegram-sidecar.md) for the first first-party adapter built against it.

Both SDKs cover the same protocol surface and pin themselves to the same conformance corpus from both directions (producer of events, consumer of commands).
The supervisor does not care which side an adapter comes from — `command = "python3 -m my_adapter"` and `command = "/usr/local/bin/my-rust-adapter"` are equally valid `[[sidecar_channels]]` entries, as is anything else that speaks the protocol.
New languages (Go, JS, …) can be added the same way; each new SDK adds an entry to the corpus's coverage matrix.

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

`librefang.sidecar.adapters.ntfy` (ships in the `librefang-sdk` Python
package; source at `sdk/python/librefang/sidecar/adapters/ntfy.py`) is
the canonical migration (#5224). It replaced the former in-process
`librefang-channels::ntfy` adapter with behaviour preserved (SSE
subscribe, `/command` parsing, `title`→sender, `topic` metadata,
chunked plain-text publish, optional Bearer auth, backoff reconnect).
`NtfyConfig` / `[channels.ntfy]` were removed and `ntfy` deleted from
the allowlist, so the gate now permanently blocks an in-process ntfy.
This was a **breaking config change**: an existing `[channels.ntfy]`
block is re-declared as a `[[sidecar_channels]]` running
`python3 -m librefang.sidecar.adapters.ntfy`. The separate ntfy
*push-notification provider*
(`push_provider = "ntfy"`) is an unrelated feature and was untouched.

## Long-tail migration backlog

ntfy proved the pipeline but also showed that fully removing one
in-process channel's config type has a wide, kernel-touching,
**breaking** blast radius (config schema, api routes/features, kernel
`channel_sender` registry, cli TUI, validation, golden). Subsequent
migrations have followed the same pattern: telegram (#5241), gotify,
mastodon, bluesky, reddit, twitch, rocketchat, discord, and now slack
— all with hand-rolled stdlib-only transports (longpoll for telegram
+ reddit, WebSocket for gotify + discord + slack, SSE for mastodon,
REST polling for bluesky + rocketchat, IRC over TLS for twitch; the
SDK has zero runtime dependencies). The discord sidecar introduced a
`select`-gated heartbeat scheduler so a single WS read loop can
interleave Discord Gateway heartbeats without a mid-frame timeout
race — the in-process Rust adapter never sent its own heartbeats,
so the sidecar is the first time discord sessions survive long idle
periods. The slack sidecar reuses the same select-gated WS pattern
for Socket Mode envelope handling. The in-process set only shrinks over time: subsequent
cleanups have dropped 12 unmaintained adapters outright (gitter,
keybase, flock, pumble, revolt, guilded, mumble, xmpp, irc, threema,
twist, voice) rather than migrating them. Anyone who still needs one
of those should ship a sidecar adapter (the same shape as the existing
SDK examples). New channels are sidecar by policy, so the in-process
set has no forced campaign — it only shrinks. Each removal or
migration is a breaking change for that channel's `[channels.<x>]`
config and must be called out in `CHANGELOG.md` under `### Changed`.
