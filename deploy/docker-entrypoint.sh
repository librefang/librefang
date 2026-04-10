#!/bin/sh
set -e

# Runs as root. Files created here must be chown'd to 'node'.

DATA_DIR="${LIBREFANG_HOME:-/data}"
CONFIG="$DATA_DIR/config.toml"

mkdir -p "$DATA_DIR"

if [ "$(stat -c '%U' "$DATA_DIR" 2>/dev/null)" != "node" ]; then
  chown -R node:node "$DATA_DIR"
fi

# First boot only. Subsequent boots skip init: the kernel re-syncs the
# registry on its own at startup (see librefang-kernel/src/kernel.rs ~2054),
# and re-running `librefang init` on every boot would accumulate timestamped
# config backups via the upgrade path.
if [ ! -f "$CONFIG" ]; then
  gosu node librefang init
fi

# Railway/Render/Fly inject PORT — reapply on every boot since a rescheduled
# machine may land on a different port.
if [ -n "$PORT" ]; then
  sed -i "s|^api_listen = .*|api_listen = \"0.0.0.0:${PORT}\"|" "$CONFIG"
  chown node:node "$CONFIG"
fi

if [ -n "$LIBREFANG_MODEL" ]; then
  sed -i "s|^model = .*|model = \"${LIBREFANG_MODEL}\"|" "$CONFIG"
  chown node:node "$CONFIG"
fi

exec gosu node "$@"
