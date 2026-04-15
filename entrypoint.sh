#!/bin/bash
# Drop root privileges and run librefang as the librefang user
chown -R librefang:librefang /data 2>/dev/null
chown -R librefang:librefang /home/librefang 2>/dev/null

# Sync WhatsApp gateway source files from the image into /data.
# Preserves runtime state (auth_store, messages.db, node_modules, logs).
# Idempotent: safe to run on every container start.
if [ -d /opt/librefang/packages/whatsapp-gateway ]; then
  mkdir -p /data/whatsapp-gateway
  # Source files (overwrite every boot so image updates propagate)
  cp -f /opt/librefang/packages/whatsapp-gateway/index.js      /data/whatsapp-gateway/index.js
  cp -f /opt/librefang/packages/whatsapp-gateway/package.json  /data/whatsapp-gateway/package.json
  cp -rf /opt/librefang/packages/whatsapp-gateway/lib          /data/whatsapp-gateway/lib
  cp -rf /opt/librefang/packages/whatsapp-gateway/scripts      /data/whatsapp-gateway/scripts 2>/dev/null || true
  # Remove stale pre-2026-04-14 bundle that shipped as index.cjs
  rm -f /data/whatsapp-gateway/index.cjs /data/whatsapp-gateway/index.cjs.hash

  # Write a deployment-tuned ecosystem.config.cjs with LIBREFANG_CONFIG env
  # pointed at the real config path. We overwrite every boot so operator
  # edits can only live in /data/whatsapp-gateway/ecosystem.override.cjs
  # (not currently read, but reserved for future).
  cat > /data/whatsapp-gateway/ecosystem.config.cjs <<'PMCONF'
module.exports = {
  apps: [{
    name: 'whatsapp-gateway',
    script: 'index.js',
    cwd: '/data/whatsapp-gateway',
    node_args: '--experimental-vm-modules',
    autorestart: true,
    max_restarts: 50,
    min_uptime: '10s',
    restart_delay: 5000,
    max_memory_restart: '256M',
    exp_backoff_restart_delay: 1000,
    error_file: '/data/whatsapp-gateway/logs/pm2-error.log',
    out_file: '/data/whatsapp-gateway/logs/pm2-out.log',
    merge_logs: true,
    time: true,
    env: {
      NODE_ENV: 'production',
      LIBREFANG_CONFIG: '/data/config.toml',
    },
  }],
};
PMCONF

  chown -R librefang:librefang /data/whatsapp-gateway/lib /data/whatsapp-gateway/scripts 2>/dev/null
  chown librefang:librefang /data/whatsapp-gateway/index.js /data/whatsapp-gateway/package.json /data/whatsapp-gateway/ecosystem.config.cjs 2>/dev/null
fi

# Resurrect PM2 processes (whatsapp-gateway etc.) before starting LibreFang.
# If the saved PM2 dump points at a stale index.cjs or a missing script, the
# restart falls back to starting the ecosystem directly — which uses the
# fresh index.js we just synced above.
gosu librefang bash -c '
  pm2 resurrect 2>/dev/null || true
  if ! pm2 describe whatsapp-gateway >/dev/null 2>&1; then
    pm2 start /data/whatsapp-gateway/ecosystem.config.cjs 2>/dev/null || true
  fi
  pm2 save 2>/dev/null || true
'

# Restore MemPalace setup from persistent /data/ volume
if [ -d /data/mempalace ]; then
  gosu librefang bash -c '
    # Config pointer
    mkdir -p ~/.mempalace
    cp -n /data/mempalace/config.json ~/.mempalace/config.json 2>/dev/null || true

    # Plugin symlink
    mkdir -p ~/.librefang/plugins
    ln -sf /data/mempalace/plugin ~/.librefang/plugins/mempalace-indexer 2>/dev/null || true

    # Rebuild venv if missing (idempotent, cached wheels make this fast)
    if [ ! -f ~/.mempalace-venv/bin/python ]; then
      ~/.local/bin/uv venv ~/.mempalace-venv --python python3 2>/dev/null
      ~/.local/bin/uv pip install mempalace --python ~/.mempalace-venv/bin/python 2>/dev/null
    fi
  '
fi

# Load secrets.env into the environment before launching the kernel.
# Upstream PR #2359 makes the kernel load this file autonomously, but we
# source it here too so older images keep working and so the env is
# already populated when `gosu` execs into the librefang user.
if [ -f /data/secrets.env ]; then
  set -a
  # shellcheck disable=SC1091
  source /data/secrets.env
  set +a
fi

exec gosu librefang librefang start --foreground
