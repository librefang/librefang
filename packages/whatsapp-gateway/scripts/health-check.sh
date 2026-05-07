#!/usr/bin/env bash
# WhatsApp Gateway Health Check
#
# Designed to run every 5 minutes from an external scheduler (LibreFang cron
# or system cron). Distinguishes between gateway-process failures (which a
# restart can fix) and network/DNS failures (which a restart cannot fix), and
# escalates to a flag file the agent can pick up.
#
# Behaviour summary:
#   1. Heartbeat: every run writes /data/whatsapp-gateway/health-check.heartbeat
#      so a missing heartbeat tells the agent the cron itself is dead.
#   2. Quick check: GET /health with a short timeout. Healthy if HTTP 200 and
#      connected:true. On success the failure counter and any flag are cleared.
#   3. On failure: increment a persistent counter. Only attempt recovery when
#      consecutive failures cross MIN_FAILURES_BEFORE_RECOVERY — this avoids
#      thrashing on a single transient blip.
#   4. Diagnose before restarting: if DNS resolution for web.whatsapp.com is
#      broken, mark the failure as ENVIRONMENTAL and skip the PM2 restart
#      (restarting won't fix DNS, and the surge of reconnects pollutes logs).
#   5. Otherwise restart via PM2 once and re-check.
#   6. After RECOVERY_GIVE_UP_FAILURES failures, write the flag file so the
#      agent surfaces it to the operator on the next heartbeat.

set -euo pipefail

GATEWAY_DIR="${GATEWAY_DIR:-/data/whatsapp-gateway}"
PORT="${WA_GATEWAY_PORT:-3009}"
HEALTH_URL="http://localhost:${PORT}/health"
HEALTH_TIMEOUT=5

LOG_FILE="${GATEWAY_DIR}/logs/health.log"
LOG_MAX_BYTES=$((1024 * 1024))            # 1 MiB before rotation (this script's own log)
PM2_LOG_MAX_BYTES=$((10 * 1024 * 1024))   # 10 MiB before rotating pm2 stdout/stderr
HEARTBEAT_FILE="${GATEWAY_DIR}/health-check.heartbeat"
COUNTER_FILE="${GATEWAY_DIR}/health-check.failures"
FLAG_FILE="${GATEWAY_DIR}/health-check-failed.flag"

MIN_FAILURES_BEFORE_RECOVERY=2            # ~10 min of disruption
RECOVERY_GIVE_UP_FAILURES=4               # ~20 min then escalate to agent
RECOVERY_WAIT=15                          # seconds to let pm2 settle

mkdir -p "$(dirname "$LOG_FILE")"

log() {
  if [[ -f "$LOG_FILE" ]] && [[ "$(stat -c %s "$LOG_FILE" 2>/dev/null || echo 0)" -gt "$LOG_MAX_BYTES" ]]; then
    mv -f "$LOG_FILE" "${LOG_FILE}.1" 2>/dev/null || true
  fi
  echo "$(date '+%Y-%m-%d %H:%M:%S') $*" >> "$LOG_FILE"
}

rotate_pm2_logs() {
  # PM2 has no built-in rotation without the pm2-logrotate module. Rather than
  # add another dependency, we cap pm2-error.log and pm2-out.log here. We keep
  # one .1 backup so a recent crash trail survives one rotation.
  local f
  for f in "${GATEWAY_DIR}/logs/pm2-error.log" "${GATEWAY_DIR}/logs/pm2-out.log"; do
    [[ -f "$f" ]] || continue
    local size
    size=$(stat -c %s "$f" 2>/dev/null || echo 0)
    if (( size > PM2_LOG_MAX_BYTES )); then
      cp -f "$f" "${f}.1" 2>/dev/null || true
      : > "$f"
      log "[rotate] truncated $(basename "$f") (was $size bytes)"
    fi
  done
}

read_counter() {
  local v
  v=$([[ -f "$COUNTER_FILE" ]] && cat "$COUNTER_FILE" 2>/dev/null || echo 0)
  # Guard against non-numeric content (manual edits, partial writes)
  # — `set -e` would crash the failure-counter arithmetic otherwise.
  [[ "$v" =~ ^[0-9]+$ ]] || v=0
  echo "$v"
}

write_counter() {
  echo "$1" > "$COUNTER_FILE"
}

clear_failure_state() {
  rm -f "$COUNTER_FILE" "$FLAG_FILE"
}

check_health() {
  local response http_code body
  response=$(curl -s --max-time "$HEALTH_TIMEOUT" -o - -w '%{http_code}' "$HEALTH_URL" 2>/dev/null) || return 1
  http_code="${response: -3}"
  body="${response%???}"

  if [[ "$http_code" != "200" ]]; then
    log "[health] HTTP $http_code (expected 200)"
    return 1
  fi

  if echo "$body" | grep -q '"connected"[[:space:]]*:[[:space:]]*true'; then
    return 0
  fi

  log "[health] Gateway responded but connected != true"
  return 1
}

dns_ok() {
  # Returns 0 if web.whatsapp.com resolves. We try getent first (no extra deps),
  # then nslookup as a fallback. If neither is present we assume DNS is fine
  # rather than mis-diagnose.
  if command -v getent >/dev/null 2>&1; then
    getent hosts web.whatsapp.com >/dev/null 2>&1 && return 0 || return 1
  fi
  if command -v nslookup >/dev/null 2>&1; then
    nslookup -timeout=3 web.whatsapp.com >/dev/null 2>&1 && return 0 || return 1
  fi
  return 0
}

pm2_restart() {
  if ! command -v pm2 >/dev/null 2>&1; then
    log "[health] pm2 binary not found in PATH"
    return 1
  fi
  pm2 restart whatsapp-gateway 2>&1 | while read -r line; do log "[pm2] $line"; done
}

write_flag() {
  local kind=$1
  local pm2_status=""
  if command -v pm2 >/dev/null 2>&1; then
    pm2_status=$(pm2 describe whatsapp-gateway 2>&1 | head -40 || true)
  fi
  cat > "$FLAG_FILE" <<EOF
timestamp=$(date -Iseconds)
kind=$kind
consecutive_failures=$(read_counter)
port=$PORT
gateway_dir=$GATEWAY_DIR
pm2_status:
$pm2_status
EOF
}

# --- Main ---

date -Iseconds > "$HEARTBEAT_FILE"
rotate_pm2_logs

# Emit the LibreFang cron pre_check_script gate. Default to "skip the agent
# turn" — we only wake the agent if a flag file gets written below, in which
# case we print an empty (or different) line and let the agent fire.
emit_skip() { echo '{"wakeAgent": false}'; }

if check_health; then
  if [[ -f "$COUNTER_FILE" ]] || [[ -f "$FLAG_FILE" ]]; then
    log "[health] Recovered after $(read_counter) consecutive failures"
  fi
  clear_failure_state
  emit_skip
  exit 0
fi

failures=$(($(read_counter) + 1))
write_counter "$failures"
log "[health] Health check failed (consecutive=$failures)"

if (( failures < MIN_FAILURES_BEFORE_RECOVERY )); then
  # First blip — wait one more cycle before reacting. Most disconnects are
  # short and self-healing. Don't wake the agent yet either.
  emit_skip
  exit 0
fi

if ! dns_ok; then
  # The gateway can't reach WhatsApp because the host can't resolve DNS.
  # Restarting node won't fix that and just clutters the logs with retries.
  log "[health] DNS resolution failed for web.whatsapp.com — environmental issue, skipping restart"
  if (( failures >= RECOVERY_GIVE_UP_FAILURES )); then
    write_flag dns-blackout
    log "[health] Wrote flag file (kind=dns-blackout)"
    # Wake the agent so the operator hears about it.
    exit 1
  fi
  emit_skip
  exit 1
fi

log "[health] Attempting PM2 restart"
pm2_restart || true
sleep "$RECOVERY_WAIT"

if check_health; then
  log "[health] Recovery succeeded after restart"
  clear_failure_state
  emit_skip
  exit 0
fi

if (( failures >= RECOVERY_GIVE_UP_FAILURES )); then
  write_flag restart-failed
  log "[health] Wrote flag file (kind=restart-failed)"
  # Wake the agent.
  exit 1
fi

emit_skip
exit 1
