#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
TEST_TMP=$(mktemp -d)
trap 'rm -rf "$TEST_TMP"' EXIT
FIXTURE="$TEST_TMP/fixture"
REPORT="$TEST_TMP/report.json"
mkdir -p "$FIXTURE/sdk/generated"

# Assemble credential-shaped values so this regression source is not itself a
# gitleaks finding. None is a real credential.
prefix=ghp_
printf 'token = "%s"\n' "${prefix}S3nt1nelMustNeverAppearInAnyResponsF" \
  > "$FIXTURE/sdk/generated/source.rs"
printf 'token = "%s"\n' "${prefix}SECRETabcdef0123456789SECRETabcdef00" \
  > "$FIXTURE/dependency.lock"
printf 'token = "%s" # dummy example\n' \
  "${prefix}abcdefghij1234567890abcdefghij123455" \
  > "$FIXTURE/context.txt"

set +e
gitleaks dir "$FIXTURE" --config "$REPO_ROOT/.gitleaks.toml" \
  --report-format json --report-path "$REPORT" --redact --no-banner
status=$?
set -e

[[ $status -eq 1 ]] || {
  echo "expected gitleaks to reject the negative allowlist fixtures, got $status" >&2
  exit 1
}
for path in sdk/generated/source.rs dependency.lock context.txt; do
  grep -q "$path" "$REPORT" || {
    echo "gitleaks did not report $path" >&2
    exit 1
  }
done
