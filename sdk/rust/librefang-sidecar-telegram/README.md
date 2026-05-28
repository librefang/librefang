# librefang-sidecar-telegram

Telegram channel adapter for [LibreFang](https://librefang.ai), packaged as a sidecar binary.
Long-polls the Bot API, translates inbound updates into LibreFang `message` events, and routes outbound LibreFang `send` / `interactive` / `reaction` / `typing` / streaming commands back to the Bot API.

Feature-parity with `sdk/python/librefang/sidecar/adapters/telegram.py` — same wire shape, same `Schema`, same access-control semantics, same emoji-reaction map.

## Build

```bash
cargo build --release -p librefang-sidecar-telegram
```

Binary lands at `target/release/librefang-sidecar-telegram`.
TLS is rustls (no system OpenSSL dependency).

## Configure

1. Get a bot token from [@BotFather](https://t.me/BotFather).
2. Add a `[[sidecar_channels]]` block to `~/.librefang/config.toml`:

```toml
[[sidecar_channels]]
name = "telegram"
command = "/abs/path/to/target/release/librefang-sidecar-telegram"
args = []
restart = true

[sidecar_channels.env]
ALLOWED_USERS = "123456789, @your_username"          # optional, empty = open
TELEGRAM_CLEAR_DONE_REACTION = "true"                # optional

[sidecar_channels.secrets]
TELEGRAM_BOT_TOKEN = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
```

Or use the dashboard's configure form — the schema is served from the adapter binary via `--describe`, so the form fields are discovered automatically.

## Capabilities

- **text** — incoming and outgoing text messages, with Markdown → Telegram HTML formatting and 4096-UTF-16-unit chunking.
- **media** — incoming and outgoing photos / documents / voice / audio / video / animation / sticker; inline file bytes (`FileData`) are multi-part uploaded.
- **media-group** — outbound `MediaGroup` with 2–10 photos / videos.
- **typing** — `Typing` command → `sendChatAction`.
- **reactions** — `Reaction` command → `setMessageReaction` with the same emoji-translation map as Python (⏳ → 👀, ⚙️ → ⚡, ✅ → 🎉 or cleared depending on `TELEGRAM_CLEAR_DONE_REACTION`, ❌ → 👎).
- **interactive** — inline-keyboard `Interactive` and `EditInteractive`; inbound `callback_query` becomes a `ButtonCallback` event.
- **thread** — forum-topic `message_thread_id` propagated end-to-end.
- **streaming** — `StreamStart` / `StreamDelta` / `StreamEnd` produce a single editable message, debounced at 1 second between edits.
- **polls** — outbound `Poll`; inbound `poll_answer` becomes a `PollAnswer` event.
- **commands** — leading `/cmd args…` text becomes a `Command` content event (with the `@botname` suffix stripped).

## Access control

The optional `ALLOWED_USERS` env var is a comma-separated list of either numeric user IDs (matched exactly) or `@usernames` (matched case-insensitively with the leading `@` optional).
Empty list ⇒ anyone may interact.
Updates from disallowed senders are dropped in the poll loop with no log line (avoids leaking sender identity into the supervisor's stderr).

## License

MIT.
