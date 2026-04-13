#!/bin/bash
set -euo pipefail

# Fix PVC ownership on every start
chown -R librefang:librefang /data

# Workaround: librefang-cli bootstrap helpers hard-code ~/.librefang.
# Since HOME=/data and LIBREFANG_HOME=/data, make /data/.librefang → /data.
# This makes ~/.librefang/config.toml resolve to /data/config.toml correctly.
ln -sfn /data /data/.librefang

# Seed Python venv if absent
if [ ! -d /data/venv ]; then
  gosu librefang python3 -m venv /data/venv
fi

# Copy config from ConfigMap mount (if present) into data tree on every start
if [ -d /etc/librefang/config ]; then
  for f in config.toml aliases.toml; do
    [ -f /etc/librefang/config/$f ] && cp /etc/librefang/config/$f /data/$f
  done
  for d in channels providers integrations; do
    [ -d /etc/librefang/config/$d ] && {
      mkdir -p /data/$d
      cp -r /etc/librefang/config/$d/. /data/$d/
    }
  done
  chown -R librefang:librefang /data
fi

# RPi systemd env parity
export VIRTUAL_ENV=/data/venv
export PATH=/data/venv/bin:/usr/local/bin:/usr/bin:/bin

cd /data && exec gosu librefang "$@"
