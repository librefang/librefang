# librefang-channels — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

Channel infrastructure crate. Every channel adapter runs
out-of-process as a sidecar (`librefang.sidecar.adapters.*` in
`sdk/python/`); this crate owns the trampoline that connects the
kernel to those sidecars (`sidecar.rs`), the shared bridge types
every adapter speaks, and the shared HTTP client.

## Cargo features

`default = []`. No feature flags. The historical `channel-*` /
`all-channels` aliases gated in-process adapters; with every
channel sidecar-migrated there is nothing left to gate.

## Always-compiled core

Every module compiles unconditionally. Modules: `attachment_enrich`,
`bridge`, `commands`, `formatter`, `group_history`, `http_client`,
`message_journal`, `message_truncator`, `rate_limiter`, `roster`,
`router`, `sanitizer`, **`sidecar`** (the trampoline), `thread_ownership`,
`types`.

## Boundary

- Owns: `ChannelAdapter` trait, `ChannelMessage` event type, the
  sidecar trampoline, and the shared bridge helpers.
- Does NOT own: kernel's per-`(agent,session)` lock (channel
  messages always derive `SessionId::for_channel(agent,"channel:chat")`).
  HTTP webhook routes — those live in
  `librefang-api/src/routes/channels.rs`.
- Depends on: `librefang-types`, `librefang-extensions` (for vault),
  `librefang-http`. NOT on `librefang-kernel` or `librefang-runtime`
  directly.

## Webhook security (mandatory)

HMAC verification is **mandatory** on every sidecar that takes
inbound webhooks — Teams, DingTalk, WhatsApp Cloud, WeChat, etc.
The verification happens inside the sidecar (see the
`librefang.sidecar.adapters.<name>` module's `_verify_request`-style
helper). Missing signature → 400. Mismatch → 401. Don't silently
bypass.

Probes without the platform's signature header (curl, monitoring
health checks) return 4xx rather than 200. That's intended.

## Outbound webhook SSRF guard

The `librefang.sidecar.adapters.webhook` sidecar's `WEBHOOK_CALLBACK_URL`
MUST resolve to a public IP. The SSRF guard lives in the Python
sidecar (pure-Python port of `http_client::validate_url_for_fetch`)
and runs both at adapter construction AND on every outbound POST.
Rejects:
- Private (10/8, 172.16/12, 192.168/16)
- CGN (100.64/10)
- Loopback (127/8, ::1)
- Link-local, multicast, cloud metadata
- IPv6 short forms ([::]), IPv4-mapped ([::ffff:127.0.0.1]), NAT64,
  trailing-dot FQDNs

Local dev: use a public tunnel (ngrok, cloudflared) or omit
`WEBHOOK_CALLBACK_URL`.

## Send-path testing

Every sidecar adapter has its own pytest suite at
`sdk/python/tests/test_<name>_adapter.py` exercising both inbound
parsing and outbound send. New send() work owes a wiremock-style
unit test in the adapter's test file.

## Adding a new channel

**Sidecar-only**. A new channel is an out-of-process sidecar
adapter, not a new module here. See `CONTRIBUTING.md` ("Add a
sidecar channel adapter"), `docs/architecture/sidecar-channels.md`,
and the existing adapters under `sdk/python/librefang/sidecar/adapters/`
for templates.

A new in-process `impl ChannelAdapter for X` (other than the
`SidecarAdapter` trampoline) is **rejected** by
`scripts/hooks/pre-commit` and `cargo xtask channel-policy` (CI)
unless the source basename is in `src/channels-allowlist.txt`.
That allowlist currently contains only `sidecar`. Adding a name
back is an explicit maintainer decision in a separate reviewed
commit, not routine.

## Taboos

- No `librefang-kernel` import. Channels are below kernel; kernel
  calls into channels through dispatch.
- No bespoke `reqwest::Client`. Use
  `librefang-extensions::http_client::shared_client()`.
- No new in-process channel modules. Sidecar-only — the
  `channels-allowlist.txt` ratchet enforces it.
- No silently bypassing HMAC verification in sidecar adapters.
  Either implement, or refuse to start.
- No SSRF-leaky `WEBHOOK_CALLBACK_URL` parsing. Use the existing
  guard in `librefang.sidecar.adapters.webhook`.
