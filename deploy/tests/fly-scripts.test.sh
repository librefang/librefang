#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
DEPLOY_SCRIPT="${ROOT}/deploy/fly/deploy.sh"
UNINSTALL_SCRIPT="${ROOT}/deploy/fly/uninstall.sh"

bash -n "$DEPLOY_SCRIPT" "$UNINSTALL_SCRIPT"

! grep -Eq 'curl[^|]*\|[[:space:]]*sh' "$DEPLOY_SCRIPT"
! grep -Eq 'flyctl secrets set.*KEY_VAL' "$DEPLOY_SCRIPT"
grep -Fq 'flyctl secrets import --app "$APP_NAME"' "$DEPLOY_SCRIPT"
grep -Fq 'read -rsp' "$DEPLOY_SCRIPT"
grep -Fq 'trap cleanup EXIT' "$DEPLOY_SCRIPT"
grep -Fq 'trap cleanup EXIT' "$UNINSTALL_SCRIPT"
grep -Fq "name == 'librefang' or name.startswith('librefang-')" "$UNINSTALL_SCRIPT"
grep -Fq 'destroy_failures=$((destroy_failures + 1))' "$UNINSTALL_SCRIPT"
