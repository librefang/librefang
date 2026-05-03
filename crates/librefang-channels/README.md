# librefang-channels

Channel Bridge Layer for the [LibreFang](https://github.com/librefang/librefang) Agent OS.

Provides 40+ pluggable messaging integrations that convert platform
messages into unified `ChannelMessage` events for the kernel and route
agent replies back out. Channels are gated behind cargo features
(`channel-xxx`).

## Cargo features

The crate has `default = []` — every workspace consumer
(`librefang-api`, `librefang-cli`, `librefang-desktop`) sets
`default-features = false` and forwards an explicit subset. Pick
features explicitly when depending on this crate:

- `all-channels` — every adapter, including heavy ones (matrix, IMAP,
  MQTT, Bluesky, Nostr, …). Used by release CI.
- Per-adapter: `channel-telegram`, `channel-discord`,
  `channel-slack`, `channel-webhook`, `channel-ntfy`, etc.

See `Cargo.toml` for the full feature list.

## Always-compiled core

`attachment_enrich`, `bridge`, `commands`, `formatter`,
`message_journal`, `message_truncator`, `rate_limiter`, `roster`,
`router`, `sanitizer`, `sidecar`, `thread_ownership`, `types`.

Useful re-exports: `split_to_utf16_chunks`,
`truncate_to_utf16_limit`, `utf16_len`, `DISCORD_MESSAGE_LIMIT`,
`TELEGRAM_CAPTION_LIMIT`, `TELEGRAM_MESSAGE_LIMIT`.

## Key dependencies

`librefang-types`, channel-specific SDKs gated per feature
(`teloxide`, `serenity`, `slack-rust`, `matrix-sdk`, `lettre`, …).

See the [workspace README](../../README.md).
