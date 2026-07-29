#!/bin/sh
set -e

# Dual-mode entrypoint (#6632).
#
# Root mode (Docker / Compose / any runtime that does not override the user):
#   the image declares no USER, so this script starts as root, can chown a
#   freshly bind-mounted or freshly created volume to uid/gid 1001, and drops
#   privileges with `gosu librefang` before exec'ing the daemon. This is the
#   historical behaviour and the migration path for existing Compose users —
#   it is unchanged.
#
# Rootless mode (Kubernetes `restricted` Pod Security):
#   the pod sets `runAsUser: 1001` / `runAsNonRoot: true`, so this script
#   starts as uid 1001. chown is impossible and gosu is unnecessary, so both
#   are skipped; the volume must already be writable by gid 1001, which the
#   pod's `fsGroup: 1001` arranges. Nothing here escalates: the same script
#   satisfies both worlds by asking what it already is, rather than shipping
#   a second image.
#
# Everything else — config initialisation, the TOML-injection guard, listen /
# model rewrites — is identical in both modes.

DATA_DIR="${LIBREFANG_HOME:-/data}"
CONFIG="$DATA_DIR/config.toml"

if [ "$(id -u)" = "0" ]; then
  ROOTLESS=0
else
  ROOTLESS=1
fi

# Run a command as the `librefang` service account.
#
# Under root that means dropping privileges through gosu. Under rootless we
# already *are* that account, and calling gosu would fail: dropping privileges
# requires the privilege to do so. Files created either way end up owned by
# uid 1001, which is the invariant the daemon actually depends on.
as_app() {
  if [ "$ROOTLESS" = "1" ]; then
    "$@"
  else
    gosu librefang "$@"
  fi
}

# Re-assert ownership of a file this script rewrote.
#
# Only meaningful under root, where the rewrite happened as uid 0 and would
# otherwise leave a root-owned config.toml the daemon cannot write. Rootless
# rewrites are already performed as uid 1001, and chown would fail.
own_as_app() {
  if [ "$ROOTLESS" = "0" ]; then
    chown librefang:librefang "$1"
  fi
}

# --- Env validation (TOML-injection guard, GH #3556) ------------------------
# The two `sed` calls below splice $PORT and $LIBREFANG_MODEL directly into
# config.toml. Without validation, an attacker controlling those env vars
# (e.g. on a `docker run -e` deployment, or a misconfigured PaaS console)
# can break out of the TOML string and inject arbitrary keys, e.g.
#   LIBREFANG_MODEL='gpt-5"\n[provider]\napi_key = "stolen'
# Reject the offending bytes here, before any rewrite happens, so a bad
# value crashes the container fast instead of silently exfiltrating config.
if [ -n "${PORT-}" ] && ! printf '%s' "$PORT" | grep -qE '^[0-9]+$'; then
  echo "ERROR: PORT must be a positive integer (got: $PORT)" >&2
  exit 1
fi
if [ -n "${LIBREFANG_MODEL-}" ]; then
  # Forbid TOML-significant characters: " \ [ ] (any one of these can
  # terminate the string or open a new table).  `case` is used (not
  # `grep`) because shell glob patterns match the literal characters
  # without surprises around backslash quoting in regex bracket
  # expressions.
  case "$LIBREFANG_MODEL" in
    *'"'*|*'\'*|*'['*|*']'*)
      echo "ERROR: LIBREFANG_MODEL contains a forbidden character (one of: \" \\ [ ])" >&2
      exit 1
      ;;
  esac
  # Embedded newlines / carriage returns can also break out of the
  # string. `grep -qE` is line-oriented, so we count newlines via
  # `wc -l` against a string printed without a trailing newline.
  if [ "$(printf '%s' "$LIBREFANG_MODEL" | wc -l | tr -d ' ')" != "0" ]; then
    echo "ERROR: LIBREFANG_MODEL must not contain newlines" >&2
    exit 1
  fi
  case "$LIBREFANG_MODEL" in
    *$(printf '\r')*)
      echo "ERROR: LIBREFANG_MODEL must not contain carriage returns" >&2
      exit 1
      ;;
  esac
fi
# ---------------------------------------------------------------------------

mkdir -p "$DATA_DIR"

if [ "$ROOTLESS" = "0" ]; then
  if [ "$(stat -c '%U' "$DATA_DIR" 2>/dev/null)" != "librefang" ]; then
    chown -R librefang:librefang "$DATA_DIR"
  fi
else
  # Rootless: we cannot take ownership, so verify up front that the volume is
  # writable and fail with an actionable message instead of letting the daemon
  # die later on an opaque "permission denied" while opening its SQLite file.
  #
  # The supported way to arrange this is the pod's `fsGroup: 1001`, which makes
  # the kubelet chgrp the volume to gid 1001 at mount time. Some CSI drivers
  # ignore fsGroup (many NFS and CIFS drivers, and any driver reporting
  # `fsGroupPolicy: None`); on those the volume must be pre-owned by 1001:1001
  # out of band. See deploy/kubernetes/README.md.
  if ! probe="$(mktemp "$DATA_DIR/.write-probe.XXXXXX" 2>/dev/null)"; then
    echo "ERROR: running rootless as uid $(id -u) but $DATA_DIR is not writable." >&2
    echo "       Set 'fsGroup: 1001' in the pod securityContext, or pre-own the" >&2
    echo "       volume as 1001:1001 if your CSI driver ignores fsGroup." >&2
    echo "       See deploy/kubernetes/README.md ('Volume ownership')." >&2
    exit 1
  fi
  rm -f "$probe"
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
as_app mkdir -p "$DATA_DIR/logs"

# First boot only. Subsequent boots skip init: the kernel re-syncs the
# registry on its own at startup (see librefang-kernel/src/kernel.rs ~2054),
# and re-running `librefang init` on every boot would accumulate timestamped
# config backups via the upgrade path.
if [ ! -f "$CONFIG" ]; then
  as_app librefang init
fi

# Railway/Render/Fly inject PORT — reapply on every boot since a rescheduled
# machine may land on a different port.
# In Docker, 127.0.0.1 is the container's own loopback and is unreachable from
# the host. Force wildcard bind unless the user has already customised it.
if grep -q '^api_listen = "127.0.0.1:' "$CONFIG" 2>/dev/null; then
  sed -i 's|^api_listen = "127.0.0.1:|api_listen = "0.0.0.0:|' "$CONFIG"
  own_as_app "$CONFIG"
fi

if [ -n "$PORT" ]; then
  sed -i "s|^api_listen = .*|api_listen = \"0.0.0.0:${PORT}\"|" "$CONFIG"
  own_as_app "$CONFIG"
fi

if [ -n "$LIBREFANG_MODEL" ]; then
  sed -i "s|^model = .*|model = \"${LIBREFANG_MODEL}\"|" "$CONFIG"
  own_as_app "$CONFIG"
fi

if [ "$ROOTLESS" = "1" ]; then
  exec "$@"
else
  exec gosu librefang "$@"
fi
