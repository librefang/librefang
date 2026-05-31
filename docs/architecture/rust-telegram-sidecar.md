# Rust Telegram sidecar adapter

`sdk/rust/librefang-sidecar-telegram/` is the first-party Telegram channel adapter built against the [Rust sidecar SDK](./rust-sidecar-sdk.md).
It is a feature-parity port of `sdk/python/librefang/sidecar/adapters/telegram.py` — same wire shapes, same `Schema`, same access-control semantics, same emoji-reaction translation map — packaged as a standalone binary.

Shipped with #5831.

Since #5936 the binary ships inside the platform release tarballs alongside the main `librefang` binary, so `librefang update` drops it into `~/.librefang/bin/librefang-sidecar-telegram` (`.exe` on Windows) with no manual `cargo build` and no runtime network download.
The daemon auto-resolves it (see [Auto-resolution](#auto-resolution) below), so the common configuration leaves `command` implicit.

## When to pick this over the Python adapter

Both adapters speak the same protocol and the supervisor cannot tell them apart.
Pick the Rust binary when:

- the host has no Python runtime (minimal container, Alpine-based image, distroless deploy);
- per-respawn startup latency matters (the Rust binary is ready in ~10 ms; the Python interpreter spends 100-300 ms on import even for an empty adapter);
- the deployment is bandwidth-constrained or memory-constrained and the ~3 MB stripped Rust binary beats a Python image plus `httpx` and friends.

Otherwise the Python adapter is fine; both ship the same capability set and the same security model.

## Configure

```toml
[[sidecar_channels]]
name = "telegram"
command = "librefang-sidecar-telegram"           # bare name ⇒ daemon auto-resolves the bundled binary
args = []
restart = true

[sidecar_channels.secrets]
TELEGRAM_BOT_TOKEN = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"

[sidecar_channels.env]
ALLOWED_USERS = "123456789, @your_username"      # optional, empty ⇒ open
TELEGRAM_CLEAR_DONE_REACTION = "true"            # optional, default false
```

### Auto-resolution

When `command` is empty or the bare stem `librefang-sidecar-telegram` (no path component), the daemon resolves it to the bundled binary, checking in order: the daemon's own executable directory, then `~/.librefang/bin/`, then PATH (the historical fallback).
An absolute or relative path, or any other program (`python3 -m …`, `uv`, …), is treated as explicit operator intent and passed through unchanged.
Resolution lives in `resolve_sidecar_command` in `crates/librefang-channels/src/sidecar.rs`.
A developer build that has not been installed via `librefang update` still points `command` at an explicit `target/release/librefang-sidecar-telegram` path.

The dashboard's configure form is populated from the schema the binary serves via `--describe`, so operators set the bot token and ALLOWED_USERS through the UI without hand-editing `config.toml`.
`TELEGRAM_BOT_TOKEN` is marked `secret` (the dashboard masks it on display); `ALLOWED_USERS` is marked `advanced`; `TELEGRAM_CLEAR_DONE_REACTION` is marked `advanced` + `bool`.

## Capabilities

Declared in the `ready` event, gated by the supervisor when relevant:

| Capability    | Inbound                                  | Outbound                              |
|---------------|------------------------------------------|---------------------------------------|
| `typing`      | —                                        | `Typing` → `sendChatAction`           |
| `reaction`    | —                                        | `Reaction` → `setMessageReaction`     |
| `interactive` | `callback_query` → `ButtonCallback`      | `Interactive` / `EditInteractive`     |
| `thread`      | `message_thread_id` carried in metadata  | `message_thread_id` end-to-end        |
| `streaming`   | —                                        | `StreamStart` / `Delta` / `End` → debounced `editMessageText` (1 s) |

The `polls` and `commands` features are wired through the standard `Content` enum (no separate capability flag); they're listed in the README for orientation.

## Architecture

```
                getUpdates long-poll (30 s server timeout)
                          │
                          ▼
                  api/client.rs::get_updates
                          │   Updates[]
                          ▼
                 translator::update_to_event
                          │
   message ──▶ message_event ──▶ extract_content + apply_reply ──▶ Value
   edited  ──▶ message_event(edited=true)
   callback ─▶ callback_event
   poll_answer ▶ poll_answer_event
                          │
                          ▼
                       emit(event)


   supervisor command ─▶ on_command ─▶ dispatch_content ─▶ api/client.rs::send_*
                                                              │
                                                              ▼
                                                          Bot API
```

The adapter is laid out in five layers:

- `api/` — Bot API client (reqwest + rustls), value types (`Update`, `Message`, `User`, all media variants), typed error.
- `format/` — Markdown → Telegram HTML converter (`markdown.rs`), HTML sanitiser with tag allowlist (`sanitize.rs`), UTF-16 chunker with tag-aware rebalancing (`chunk.rs`).
- `translator.rs` — inbound `Update` → `MessageBuilder`-shaped `Value` event.
- `dispatcher.rs` — outbound `Content` → Bot API call (Text, Image, File, FileData, Voice, Video, Audio, Animation, Sticker, Location, Command, Interactive, EditInteractive, DeleteMessage, MediaGroup, Poll).
- `adapter.rs` — the `TelegramAdapter` impl: produce-side long-poll, on_send / on_command dispatch, streaming-edit state map.

### Inbound

1. `produce()` long-polls `getUpdates` with `allowed_updates = ["message", "edited_message", "callback_query", "poll_answer"]`.
2. For each update, the access-control gate extracts a sender (from `message.from`, `message.sender_chat`, `callback_query.from`, or `poll_answer.user`) and checks `AllowList::permits(user_id, username)`.
   Disallowed updates are silently skipped (no log line so the supervisor's stderr never carries sender identity).
3. `update_to_event` dispatches by update kind and emits one `message`-shaped event.
   Media payloads call `getFile` to resolve the `file_id` to a public URL; on `getFile` failure the path falls back to a `[Photo received: <cap>]` / `[Document received: <filename>]` / `[Voice message, Ns]` text placeholder matching the Python adapter byte-for-byte, so the user's caption survives even when the URL doesn't.
4. `edited_message` updates emit with `metadata.edited = true` plus `metadata.edit_date` (when Telegram provides it) so the supervisor can dedupe or treat as an edit; without this they would be indistinguishable from a fresh turn (Telegram reuses the original message_id).

### Outbound

`dispatch_content` matches the externally-tagged `ChannelContent` JSON shape and calls the appropriate Bot API method.
Captioned media (Image / Voice / Video / Audio / Animation) run captions through the same `format_and_sanitize` pipeline as text, with a `can't parse entities` plain-text fallback so a malformed sanitiser output never silently drops the media.
MediaGroup is atomic on Telegram's side: 1-item groups fall back to single sends, > 10-item groups are chunked into batches of 10, and nested MediaGroups (recursive payloads) are rejected before the recursion can happen.

### Streaming

A single placeholder message (`…`) is sent on `StreamStart`; subsequent `StreamDelta` events accumulate into a per-stream buffer and edit that placeholder via `editMessageText`, debounced to one edit per second.
`StreamEnd` flushes the final buffer.
Edit failures are silently tolerated for `message is not modified` (debounce ticked without new content) and fall back to plain text on `can't parse entities`; everything else is logged.

## Text-rendering pipeline

```
raw text  ──▶ markdown_to_telegram_html  ──▶ sanitize_telegram_html  ──▶ split_to_utf16_chunks  ──▶ sendMessage(parse_mode=HTML)
                                                                                                       │
                                                                                                       └─ on 400 "can't parse entities":
                                                                                                            html_to_plain(chunk) ──▶ sendMessage(parse_mode=None)
```

- **Markdown subset.** Only the constructs the Python adapter supports — code fences, headings (`#` through `######`), blockquotes, ordered / unordered lists, bold (`**…**`), italic (`*…*`), inline code (`` `…` ``), links (`[label](url)`).
  Inline-code placeholders use Private-Use-Area sentinels (U+E000 / U+E001) that `escape_html` strips from input, so an adversarial user message containing those bytes cannot collide with the placeholder scheme and inject `<code>` past the sanitiser's tag allowlist.
- **HTML sanitiser.** Allowlist of `b`, `i`, `u`, `s`, `em`, `strong`, `a`, `code`, `pre`, `blockquote`, `tg-spoiler`, `tg-emoji` — matches Telegram's documented HTML subset.
  `<a href>` is enforced against `https:` / `http:` / `mailto:` / `tg:` schemes; anything else (including `javascript:` / `data:`) drops the tag entirely.
  Unclosed tags are auto-balanced at end-of-input.
- **UTF-16 chunker (4096-unit Telegram limit).** Telegram counts code units, not bytes or Unicode scalars; non-BMP characters count as 2.
  The chunker is tag-aware: an `<a href="…">` opened in one chunk has matching `</a>` appended AND `<a href="…">` re-emitted at the start of the next chunk, with the full attribute string preserved, so the user's formatting carries across boundaries.
  Mid-tag and mid-entity boundaries (where the cut would land inside `<…>` or `&…;`) back off to before the open `<` or `&`.

## Security

- **Bot token.** Stored in `BotClient.api_root` / `file_root` (baked into the request URL).
  Any error path that returns the URL or response body to the operator goes through `BotClient::redact(s)` which replaces the literal token with `[REDACTED]`; `From<reqwest::Error>` strips the URL entirely via `e.without_url()` before constructing `Error::Http`.
  Logs, protocol error events, and `Display` impls never leak the token.
- **Allowlist.** `AllowList::permits(user_id, username)` checks numeric IDs by exact match and `@usernames` case-insensitively (with optional leading `@`).
  Empty allowlist ⇒ open; the gate runs against every inbound event kind including `poll_answer` (skipping it would have let any Telegram user vote in the bot's polls and have the PollAnswer event reach the agent).
- **MediaGroup recursion.** Each `MediaGroup.items[i]` is checked for a `MediaGroup` key before any recursive dispatch; the heap-allocated future stack cannot be blown by an adversarial nested payload.
- **FileData byte decode.** The JSON `data` array is decoded strictly: any element that is not a non-negative integer in `[0, 255]` produces `Error::Other` rather than silently dropping or truncating.

## Rate-limiting and retry

- `call_json` and `send_multipart` retry once on a 429 Too-Many-Requests using the server-supplied `retry_after`, capped at `MAX_RETRY_AFTER_SECS = 300` so a multi-hour flood-wait surfaces as an error instead of stalling the produce loop indefinitely.
- Beyond `MAX_RETRY_AFTER_SECS`, the supervisor can choose to restart the subprocess (which clears any rate-limited state on the Bot API side via fresh tokens? — no, tokens are stable; the restart simply makes the operator-visible failure mode loud rather than silent).
- The long-poll loop in `produce` applies its own exponential backoff (1 s → 300 s cap) on non-timeout `getUpdates` failures, distinct from the per-call retry inside `call_json`.

## Python-parity deltas

The port is feature-parity by intent but has three documented deliberate divergences:

1. **`parse_command` uses `MessageEntity.length` (UTF-16) instead of `txt.split(" ", 1)`.**
   `/help:foo` returns `("help", [":foo"])` in Rust (Bot-API-correct); Python returns `("help:foo", [])` (a long-standing parser bug).
2. **`MediaGroup` with > 10 items is chunked into batches of 10.** Python raises `ValueError`.
3. **`channel`-type chats are not treated as group.** Both adapters now use `is_group = chat_type in {group, supergroup}`; the original Rust draft included `channel`, the parity fix narrowed it.

Cross-language tests pin the rest of the wire shape (`media_placeholder_matches_python_labels` covers all eight placeholder variants byte-for-byte).

## Verification

In the project's sanctioned dev container (`Dockerfile.rust-dev`):

```bash
# Build, test, lint inside named-volume cargo target so it does not contend with the host's main worktree.
docker build -t librefang-rust-dev:latest -f Dockerfile.rust-dev .
docker run --rm \
  -v "$(git rev-parse --show-toplevel)":/work \
  -v librefang-cargo:/cargo -v librefang-target:/target \
  -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
  -w /work/sdk/rust/librefang-sidecar-telegram \
  librefang-rust-dev:latest \
  sh -c 'export PATH=/usr/local/cargo/bin:$PATH; cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check'
```

40 unit tests, zero clippy warnings, formatter clean.
Live LLM testing (against a real Bot API + `librefang start`) is human-only — see CLAUDE.md `MANDATORY: Integration Testing` for the exact procedure.

## See also

- [`rust-sidecar-sdk.md`](./rust-sidecar-sdk.md) — the SDK this adapter is built against.
- [`sidecar-channels.md`](./sidecar-channels.md) — supervisor / process model / config.
- [`sidecar-protocol.md`](./sidecar-protocol.md) — wire spec, conformance corpus.
- `sdk/python/librefang/sidecar/adapters/telegram.py` — the parity reference.
