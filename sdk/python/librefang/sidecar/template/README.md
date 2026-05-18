# Sidecar channel adapter template

Scaffold for a new LibreFang sidecar channel adapter.

> **Prerequisite:** this SDK speaks the post-#5219 sidecar protocol,
> but degrades cleanly on the current `main` (minimal `text`-only
> protocol): the `ready` re-announce self-bounds, and plain-text
> `content` is mirrored into `text` so messages aren't delivered
> empty. Rich (`content`-only non-text) features need #5219 (P0–P3
> channel parity) merged for full end-to-end behaviour.

1. Copy `adapter.py.tmpl` to `adapter.py` and replace `<PLATFORM>`.
2. `pip install -r requirements.txt`
3. Implement `on_send` (deliver to your platform) and `produce`
   (push inbound platform messages via `emit`).
4. Declare `capabilities` for the rich features you support
   (`typing`, `reaction`, `interactive`, `thread`, `streaming`,
   `typing_events`). Anything you don't declare degrades to plain
   text — no code needed.
5. Register it in `~/.librefang/config.toml` under
   `[[sidecar_channels]]` (see `librefang.toml.example`).

## Rules

- **stdout is the protocol.** Never `print()` to stdout. Log via
  `from librefang.sidecar import logging` (writes stderr).
- **Process restart is the daemon's job**; **platform reconnect is
  yours** (`with_backoff`). Be crash-safe — the framework re-announces
  `ready` automatically on every fresh start.
- Tolerate unknown commands (the SDK already does — they arrive as
  `UnknownCommand`).
