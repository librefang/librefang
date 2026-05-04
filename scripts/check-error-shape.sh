#!/usr/bin/env bash
# check-error-shape.sh — guard the canonical `ApiErrorResponse` envelope.
#
# Issue #3505: the HTTP API once returned errors in four different shapes
# (`{"error": …}`, `{"detail": …}`, `{"status": "error", "message": …}`,
# and the OpenAI-compat `{"error": {message,type,code}}`). Three of the
# four are being unified onto `ApiErrorResponse` (`{"error": "<string>"}`,
# defined in `crates/librefang-api/src/types.rs`).
#
# This script grep-rejects new occurrences of the two ad-hoc shapes from
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
  "crates/librefang-api/src/routes/skills.rs"
  # Follow-up to #3505: migrate users {"status":"error", …} sites.
  "crates/librefang-api/src/routes/users.rs"
  # Follow-up to #3505: migrate config {"status":"error", …} sites.
  "crates/librefang-api/src/routes/config.rs"
)

# Build an extended-regex alternation of legacy paths for grep filtering.
LEGACY_RE=""
for f in "${LEGACY_FILES[@]}"; do
  if [ -z "$LEGACY_RE" ]; then
    LEGACY_RE="^${f}:"
  else
    LEGACY_RE="${LEGACY_RE}|^${f}:"
  fi
done

# Prefer ripgrep (fast). Fall back to grep -RHn so the script still
# works in stripped-down CI containers that don't bundle ripgrep.
if command -v rg >/dev/null 2>&1; then
  search_multi() { rg --no-heading --line-number -U "$@"; }
  search_line()  { rg --no-heading --line-number "$@"; }
else
  search_multi() { grep -RHnP "$@"; }
  search_line()  { grep -RHnE "$@"; }
fi

# Filter out hits in the legacy allowlist. Using `grep -Ev` keeps the
# script readable; the alternation regex is anchored on `^path:` so a
# substring match in a comment cannot mask hits in other files.
filter_legacy() {
  if [ -z "$LEGACY_RE" ]; then
    cat
  else
    grep -Ev "$LEGACY_RE" || true
  fi
}

violations=0

# Pattern 1: `{"detail": …}` single-key error wrapper inside `json!({…})`.
#
# `audit.rs` and similar files emit `"detail": …` as a *data* field on
# AuditEntry rows. The error-wrapper pattern always sits inside `json!({…})`
# with `"detail"` as the first key. The cheapest reliable heuristic:
# require the literal `json!({"detail":` so the AuditEntry row case
# (`json!({ "seq": …, "detail": …, … })`) is naturally excluded.
detail_hits=$(search_multi 'json!\(\{\s*"detail"\s*:' "$ROUTES_DIR" 2>/dev/null || true)
detail_hits_filtered=$(printf '%s\n' "$detail_hits" | filter_legacy | grep -Ev ':[[:space:]]*///' | sed '/^$/d' || true)
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
status_hits=$(search_line '"status"[[:space:]]*:[[:space:]]*"error"' "$ROUTES_DIR" 2>/dev/null || true)
status_hits_filtered=$(printf '%s\n' "$status_hits" | filter_legacy | grep -Ev ':[[:space:]]*///' | sed '/^$/d' || true)
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
