"""First-party sidecar channel adapters shipped with LibreFang.

Each adapter is a runnable module — invoke via ``python -m
librefang.sidecar.adapters.<name>`` from a ``[[sidecar_channels]]``
config block. See each module's header for required env vars.

Available adapters:

* :mod:`librefang.sidecar.adapters.ntfy` — ntfy.sh (SSE in / HTTP out,
  stdlib-only; replaces the removed in-process ``librefang-channels::ntfy``)
* :mod:`librefang.sidecar.adapters.telegram` — Telegram Bot API
  (long-poll; requires ``requests``)
* :mod:`librefang.sidecar.adapters.webhook` — generic inbound HTTP
  webhook receiver (stdlib-only)
"""
