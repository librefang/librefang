#!/bin/sh
set -e

# Runs as root. Files created here must be chown'd to 'librefang'.

DATA_DIR="${LIBREFANG_HOME:-/data}"
CONFIG="$DATA_DIR/config.toml"

mkdir -p "$DATA_DIR"

if [ "$(stat -c '%U' "$DATA_DIR" 2>/dev/null)" != "librefang" ]; then
  chown -R librefang:librefang "$DATA_DIR"
fi

# Pre-create the logs directory so `librefang start --foreground` can open
# its daily log file on a fresh container. The CLI also creates this dir
# itself (see setup_foreground_tee), but we do it here too as defense in
# depth — a missing logs dir previously caused the daemon to panic with
# exit 101 silently (GH #3058).
#
# Create as the librefang user so that on reused volumes (where $DATA_DIR is
# already owned by librefang and the chown -R above is skipped) the new dir
# isn't left as root:root 0755 — that would block `gosu librefang librefang`
# from writing daemon-*.log and reproduce the same failure under a
# different error code.
gosu librefang mkdir -p "$DATA_DIR/logs"

# First boot only. Subsequent boots skip init: the kernel re-syncs the
# registry on its own at startup (see librefang-kernel/src/kernel.rs ~2054),
# and re-running `librefang init` on every boot would accumulate timestamped
# config backups via the upgrade path.
if [ ! -f "$CONFIG" ]; then
  gosu librefang librefang init
fi

# Railway/Render/Fly inject PORT — reapply on every boot since a rescheduled
# machine may land on a different port.
# In Docker, 127.0.0.1 is the container's own loopback and is unreachable from
# the host. Force wildcard bind unless the user has already customised it.
if grep -q '^api_listen = "127.0.0.1:' "$CONFIG" 2>/dev/null; then
  sed -i 's|^api_listen = "127.0.0.1:|api_listen = "0.0.0.0:|' "$CONFIG"
  chown librefang:librefang "$CONFIG"
fi

if [ -n "$PORT" ]; then
  sed -i "s|^api_listen = .*|api_listen = \"0.0.0.0:${PORT}\"|" "$CONFIG"
  chown librefang:librefang "$CONFIG"
fi

if [ -n "$LIBREFANG_MODEL" ]; then
  sed -i "s|^model = .*|model = \"${LIBREFANG_MODEL}\"|" "$CONFIG"
  chown librefang:librefang "$CONFIG"
fi

exec gosu librefang "$@"
