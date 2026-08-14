#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
HEALTH_SCRIPT="${SCRIPT_DIR}/../scripts/health-check.sh"
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
MOCK_BIN="${TEST_DIR}/bin"
mkdir -p "$MOCK_BIN" "${TEST_DIR}/gateway"

cat > "${MOCK_BIN}/getent" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
cat > "${MOCK_BIN}/nslookup" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "${MOCK_BIN}/curl" <<'EOF'
#!/usr/bin/env bash
out=''
while (($#)); do
  if [[ "$1" == '-o' ]]; then out=$2; shift 2; else shift; fi
done
printf '{"connected":false,"suffix":"200"}' > "$out"
printf '503'
EOF
cat > "${MOCK_BIN}/pm2" <<'EOF'
#!/usr/bin/env bash
echo 'restart failed'
exit 7
EOF
cat > "${MOCK_BIN}/flock" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "${MOCK_BIN}"/*

PATH="${MOCK_BIN}:/usr/bin:/bin" GATEWAY_DIR="${TEST_DIR}/gateway" HEALTH_CHECK_LIB_ONLY=1 \
  bash -c 'source "$1"; dns_ok; ! check_health; if pm2_restart; then exit 1; else [[ $? -eq 7 ]]; fi' _ "$HEALTH_SCRIPT"

# A fourth DNS failure writes a flag and explicitly wakes the agent.
printf '3\n' > "${TEST_DIR}/gateway/health-check.failures"
cat > "${MOCK_BIN}/curl" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
cat > "${MOCK_BIN}/nslookup" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
chmod +x "${MOCK_BIN}/curl" "${MOCK_BIN}/nslookup"
set +e
output=$(PATH="${MOCK_BIN}:/usr/bin:/bin" GATEWAY_DIR="${TEST_DIR}/gateway" bash "$HEALTH_SCRIPT")
status=$?
set -e
[[ $status -eq 1 ]]
[[ "$output" == '{"wakeAgent": true}' ]]
grep -q '^kind=dns-blackout$' "${TEST_DIR}/gateway/health-check-failed.flag"
