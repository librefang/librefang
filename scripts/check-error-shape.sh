#!/usr/bin/env bash
# check-error-shape.sh — guard the canonical `ApiErrorResponse` envelope.
#
# Issue #3505: the HTTP API once returned errors in four different shapes
# (`{"error": …}`, `{"detail": …}`, `{"status": "error", "message": …}`,
# and the OpenAI-compat `{"error": {message,type,code}}`). Three of the
# four are being unified onto `ApiErrorResponse` (`{"error": "<string>"}`,
# defined in `crates/librefang-api/src/types.rs`).
#
# This script rejects new occurrences of the two ad-hoc shapes from
# coming back into route handlers. It enforces the rule on already-clean
# files. Files that still carry legacy shapes are listed in
# LEGACY_FILES below with their cleanup tracking issue, and are exempt
# until that follow-up lands. New files MUST be clean from day one.
#
# Allowed permanent exception:
# - `crates/librefang-api/src/openai_compat.rs` — OpenAI SDK contract.
#   Lives outside `routes/`, so it is naturally out of scope here.
#
# Exit codes:
#   0  clean
#   1  forbidden shape introduced into an enforced file
#   2  invocation error (run from outside a git checkout, etc.)

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

ROUTES_DIR="crates/librefang-api/src/routes"

if [ ! -d "$ROUTES_DIR" ]; then
  echo "::error::$ROUTES_DIR not found — run from a librefang checkout." >&2
  exit 2
fi

# Files that still carry legacy `{"status": "error", …}` or `{"detail": …}`
# error shapes at the time of #3505 and are out of scope for this PR. Each
# entry should reference a follow-up cleanup ticket. The lint skips these
# files; new violations elsewhere fail the build.
#
# Whenever a follow-up migrates one of these files, drop it from this list
# so the lint starts enforcing the rule on it permanently.
LEGACY_FILES=(
  # Follow-up to #3505: migrate provider {"status":"error", …} sites.
  "crates/librefang-api/src/routes/providers.rs"
  # Follow-up to #3505: migrate webhook {"status":"error", …} sites.
  "crates/librefang-api/src/routes/webhooks.rs"
  # Follow-up to #3505: migrate approvals {"status":"error", …} sites.
  # Note: approvals carries per-row `status` inside batch result arrays —
  # those are data fields, not error wrappers, but the lint cannot tell
  # them apart cheaply. The cleanup pass needs human review.
  "crates/librefang-api/src/routes/approvals.rs"
  # Follow-up to #3505: migrate skills {"status":"error", …} sites.
  # (routes/skills.rs was split into routes/skills/; the legacy sites now
  # live in the hands/ and mcp/ submodules — #3505 cleanup still pending.)
  "crates/librefang-api/src/routes/skills/hands.rs"
  "crates/librefang-api/src/routes/skills/mcp.rs"
  # Follow-up to #3505: migrate users {"status":"error", …} sites.
  "crates/librefang-api/src/routes/users.rs"
  # Follow-up to #3505: migrate config {"status":"error", …} sites.
  # (routes/config.rs was split into routes/config/; the legacy sites now
  # live in the manage/ and system/ submodules — #3505 cleanup still pending.)
  "crates/librefang-api/src/routes/config/manage.rs"
  "crates/librefang-api/src/routes/config/system.rs"
)

# Scan complete Rust source files so rustfmt line wrapping cannot hide a
# forbidden first JSON key. Python is already required by the repository's CI
# and gives every platform the same multiline-regex semantics.
search_routes() {
  local shape=$1
  if ! command -v python3 >/dev/null 2>&1; then
    echo "::error::python3 is required for multiline error-shape scanning." >&2
    return 2
  fi
  python3 - "$ROUTES_DIR" "$shape" <<'PY'
import os
import re
import sys

root, shape = sys.argv[1:]
patterns = {
    "detail": re.compile(r'json\s*!\s*\(\s*\{\s*"detail"\s*:', re.DOTALL),
    "status_error": re.compile(
        r'json\s*!\s*\(\s*\{\s*"status"\s*:\s*"error"', re.DOTALL
    ),
}
try:
    pattern = patterns[shape]
    for directory, directories, filenames in os.walk(root, followlinks=False):
        directories.sort()
        for filename in sorted(filenames):
            if not filename.endswith(".rs"):
                continue
            path = os.path.join(directory, filename)
            if os.path.islink(path):
                continue
            with open(path, encoding="utf-8", errors="replace") as source:
                text = source.read()
            lines = text.splitlines()
            for match in pattern.finditer(text):
                line_number = text.count("\n", 0, match.start()) + 1
                excerpt = lines[line_number - 1].strip() if lines else ""
                print(f"{path}:{line_number}:{excerpt}")
except (OSError, KeyError) as error:
    print(f"::error::error-shape search failed: {error}", file=sys.stderr)
    raise SystemExit(2)
PY
}

# Filter out hits in the legacy allowlist by exact path, never by regex.
filter_legacy() {
  local line path legacy skip
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    path=${line%%:*}
    skip=0
    for legacy in "${LEGACY_FILES[@]}"; do
      if [ "$path" = "$legacy" ]; then
        skip=1
        break
      fi
    done
    [ "$skip" = 1 ] || printf '%s\n' "$line"
  done
}

violations=0

# Pattern 1: `{"detail": …}` single-key error wrapper inside `json!({…})`.
#
# `audit.rs` and similar files emit `"detail": …` as a *data* field on
# AuditEntry rows. The error-wrapper pattern always sits inside `json!({…})`
# with `"detail"` as the first key. The cheapest reliable heuristic:
# require the literal `json!({"detail":` so the AuditEntry row case
# (`json!({ "seq": …, "detail": …, … })`) is naturally excluded.
if ! detail_hits=$(search_routes detail); then
  exit 2
fi
detail_hits_filtered=""
if [ -n "$detail_hits" ]; then
  detail_hits_filtered=$(printf '%s\n' "$detail_hits" \
    | filter_legacy \
    | grep -Ev ':[0-9]+:[[:space:]]*///' || true)
fi
if [ -n "$detail_hits_filtered" ]; then
  echo "::error::Found forbidden '{\"detail\": …}' error shape (issue #3505):"
  echo "$detail_hits_filtered"
  echo
  echo "Use \`crate::types::ApiErrorResponse\` (\`{\"error\": …}\`) instead."
  echo "Constructors: ApiErrorResponse::not_found / bad_request / forbidden /"
  echo "conflict / internal — see crates/librefang-api/src/types.rs."
  violations=$((violations + 1))
fi

# Pattern 2: `{"status": "error", …}` shape. Strip out `///` doc-comment
# lines so a doc string explaining why the shape is gone doesn't trip
# the lint.
if ! status_hits=$(search_routes status_error); then
  exit 2
fi
status_hits_filtered=""
if [ -n "$status_hits" ]; then
  status_hits_filtered=$(printf '%s\n' "$status_hits" \
    | filter_legacy \
    | grep -Ev ':[0-9]+:[[:space:]]*///' || true)
fi
if [ -n "$status_hits_filtered" ]; then
  echo "::error::Found forbidden '{\"status\": \"error\", …}' shape (issue #3505):"
  echo "$status_hits_filtered"
  echo
  echo "Use \`crate::types::ApiErrorResponse\` instead. The HTTP status code"
  echo "is the source of truth for error vs ok — clients should not branch"
  echo "on a body field. See crates/librefang-api/src/types.rs."
  violations=$((violations + 1))
fi

if [ "$violations" -gt 0 ]; then
  echo
  echo "If a new file genuinely needs one of these shapes (e.g. an external"
  echo "contract like OpenAI compat), add it to LEGACY_FILES at the top of"
  echo "this script along with the tracking issue."
  exit 1
fi

echo "OK: no new forbidden error shapes under $ROUTES_DIR (legacy files exempt: ${#LEGACY_FILES[@]})."
