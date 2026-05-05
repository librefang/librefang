# librefang-channels — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

40+ pluggable messaging integrations. Convert platform messages into unified `ChannelMessage` events for the kernel; route agent replies back out.
Adapters are gated behind cargo features (`channel-xxx`).

## Cargo features

`default = []`. Every workspace consumer (`librefang-api`, `librefang-cli`, `librefang-desktop`) sets `default-features = false` and forwards an explicit subset.

- `all-channels` — every adapter (matrix, IMAP, MQTT, Bluesky, Nostr, …). Used by release CI.
- Per-adapter: `channel-telegram`, `channel-discord`, `channel-slack`, `channel-webhook`, `channel-ntfy`, etc.

See `Cargo.toml` for the full feature matrix.

## Always-compiled core

The trait + dispatch glue compiles unconditionally. Only adapters are feature-gated.

## Boundary

- Owns: `ChannelAdapter` trait, `ChannelMessage` event type, every adapter under `src/<channel>/`.
- Does NOT own: kernel's per-`(agent,session)` lock (channel messages always derive `SessionId::for_channel(agent,"channel:chat")`). HTTP webhook routes — those live in `librefang-api/src/routes/channels.rs`.
- Depends on: `librefang-types`, `librefang-extensions` (for vault), `librefang-http`. NOT on `librefang-kernel` or `librefang-runtime` directly.

## Webhook security (mandatory)

HMAC verification is **mandatory** for Messenger, LINE, Teams, Viber, DingTalk. Missing signature → 400. Mismatch → 401. Don't silently bypass.

- Messenger: `MESSENGER_APP_SECRET` (Facebook App Secret). New `app_secret_env` in `[channels.messenger]`.
- Teams: `TEAMS_SECURITY_TOKEN` (base64 outgoing-webhook security token). New `security_token_env` in `[channels.teams]`.
- LINE / Viber / DingTalk: platform-specific signature header.

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

1. New file `src/<channel>/mod.rs` implementing `ChannelAdapter`.
2. New cargo feature `channel-<name>` in `Cargo.toml`.
3. Default-feature decision: every channel ships off-by-default. The `all-channels` feature aggregates them.
4. Wire HTTP webhook (if needed) in `librefang-api/src/routes/channels.rs`.
5. Add a `tests/<channel>_wiremock.rs` covering at least the send() happy path + one error response.
6. Document any required env vars in the adapter's doc comment.

## Taboos

- No `librefang-kernel` import. Channels are below kernel; kernel calls into channels through dispatch.
- No bespoke `reqwest::Client`. Use `librefang-extensions::http_client::shared_client()`.
- No `default = ["all-channels"]`. The default is and stays empty.
- No silently bypassing HMAC verification. Either implement, or refuse to start.
- No SSRF-leaky `callback_url` parsing. Use the existing guard.
