# Sidecar channel adapter template

Scaffold for a new LibreFang sidecar channel adapter.

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
