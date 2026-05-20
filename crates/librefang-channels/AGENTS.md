# librefang-channels — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

40+ pluggable messaging integrations. Convert platform messages into unified `ChannelMessage` events for the kernel; route agent replies back out.
Adapters are gated behind cargo features (`channel-xxx`).

## Cargo features

`default = []`. Every workspace consumer (`librefang-api`, `librefang-cli`, `librefang-desktop`) sets `default-features = false` and forwards an explicit subset.

- `all-channels` — every adapter (matrix, IMAP, google-chat, …). Used by release CI.
- Per-adapter: `channel-discord`, `channel-slack`, `channel-webhook`, etc. (ntfy, telegram, gotify, mastodon, bluesky, reddit, twitch, and rocketchat migrated to sidecars — see `librefang.sidecar.adapters.{ntfy,telegram,gotify,mastodon,bluesky,reddit,twitch,rocketchat}` in the SDK.)

See `Cargo.toml` for the full feature matrix.

## Always-compiled core

The trait + dispatch glue compiles unconditionally. Only adapters are feature-gated.

## Boundary

- Owns: `ChannelAdapter` trait, `ChannelMessage` event type, every adapter under `src/<channel>/`.
- Does NOT own: kernel's per-`(agent,session)` lock (channel messages always derive `SessionId::for_channel(agent,"channel:chat")`). HTTP webhook routes — those live in `librefang-api/src/routes/channels.rs`.
- Depends on: `librefang-types`, `librefang-extensions` (for vault), `librefang-http`. NOT on `librefang-kernel` or `librefang-runtime` directly.

## Webhook security (mandatory)

HMAC verification is **mandatory** for LINE, Teams, DingTalk. Missing signature → 400. Mismatch → 401. Don't silently bypass.

- Teams: `TEAMS_SECURITY_TOKEN` (base64 outgoing-webhook security token). New `security_token_env` in `[channels.teams]`.
- LINE / DingTalk: platform-specific signature header.

Probes without the platform's signature header (curl, monitoring health checks) now return 4xx rather than 200. That's intended.

## Outbound webhook SSRF guard

`[channels.webhook] callback_url` MUST resolve to a public IP. Adapters refuse to start if the URL points at:
- Private (10/8, 172.16/12, 192.168/16)
- CGN (100.64/10)
- Loopback (127/8, ::1)
- Link-local, multicast, cloud metadata
- IPv6 short forms ([::]), IPv4-mapped ([::ffff:127.0.0.1]), NAT64, trailing-dot FQDNs

Local dev: use a public tunnel (ngrok, cloudflared) or omit `callback_url`.

## Send-path testing

Inbound parsing has 795 tests. Outbound `send()` has historically had ~zero (#3820). New send() work MUST include a wiremock'd test in `tests/<channel>_wiremock.rs`. PRs that add an adapter without a `send()` test will be sent back.

## Adding a new channel

Sidecar-first. A new channel is an out-of-process sidecar adapter, not
a new module here. See `CONTRIBUTING.md` ("Add a sidecar channel
adapter"), `docs/architecture/sidecar-channels.md`, and the
`librefang.sidecar` SDK (`sdk/python/`).

A new in-process `impl ChannelAdapter` is **rejected** by
`scripts/hooks/pre-commit` and `cargo xtask channel-policy` (CI) unless
its basename is in `src/channels-allowlist.txt` — that list only
shrinks (a sidecar migration deletes the module and its line). Adding
a name back is an explicit maintainer decision in a separate reviewed
commit, not routine.

The grandfathered in-process adapters still obey the existing rules:
new `send()` work owes a `tests/<channel>_wiremock.rs` (happy path +
one error); HTTP webhooks wire through
`librefang-api/src/routes/channels.rs`; channels ship off-by-default
behind `channel-<name>`; required env vars go in the adapter's doc
comment.

## Taboos

- No `librefang-kernel` import. Channels are below kernel; kernel calls into channels through dispatch.
- No bespoke `reqwest::Client`. Use `librefang-extensions::http_client::shared_client()`.
- No `default = ["all-channels"]`. The default is and stays empty.
- No silently bypassing HMAC verification. Either implement, or refuse to start.
- No SSRF-leaky `callback_url` parsing. Use the existing guard.
