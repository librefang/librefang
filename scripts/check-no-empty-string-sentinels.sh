#!/usr/bin/env bash
# check-no-empty-string-sentinels.sh — Lint for empty-string sentinel patterns.
#
# Refs #3302 (1/N): API responses must not use empty strings, "<unknown>",
# "<empty>", or "none" as sentinel values for "field is unset". Use
# Option<T> with `null` (or omit via `#[serde(skip_serializing_if = ...)]`)
# instead. Ambiguity between "set to empty" and "unset" forces every client
# to special-case it and breaks the OpenAPI/typed-SDK contract.
#
# This script is INFORMATIONAL by default (exit 0) so existing offenders
# don't block PRs. Pass `--strict` to enforce zero hits in CI once the
# inventory is cleared.
#
# Scope: Rust source under
#   - crates/librefang-api/src/routes/
#   - crates/librefang-channels/src/
# Add new directories to SCAN_PATHS as the typed-API frontier expands.
#
# False positives are expected — `is_empty()` has many legitimate uses
# (validation, length checks, etc.). The reviewer judges each hit. Pre-
# existing benign usages should be allowlisted via the inline marker
# `// allow-empty-sentinel: <reason>` on the same line.
#
# Usage:
#   scripts/check-no-empty-string-sentinels.sh           # warn mode
#   scripts/check-no-empty-string-sentinels.sh --strict  # fail on any hit

set -u
# NOTE: no `set -e` — we want to scan everything and report.

REPO_ROOT="${LIBREFANG_SENTINEL_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$REPO_ROOT"

STRICT=0
for arg in "$@"; do
    case "$arg" in
        --strict) STRICT=1 ;;
        -h|--help)
            cat <<'EOF'
Usage:
  scripts/check-no-empty-string-sentinels.sh
  scripts/check-no-empty-string-sentinels.sh --strict

Scans Rust sources under API routes and channels for textual empty-value
sentinels. Default mode inventories hard and soft signals. Strict mode fails
only on hard textual/default sentinels; generic `.is_empty()` and
`.unwrap_or_default()` calls remain review-only signals.
EOF
            exit 0
            ;;
        *) echo "Unknown argument: $arg" >&2; exit 2 ;;
    esac
done

SCAN_PATHS=(
    "crates/librefang-api/src/routes"
    "crates/librefang-channels/src"
)

# Verify scan paths exist (catches accidental rename in a refactor).
for p in "${SCAN_PATHS[@]}"; do
    if [ ! -d "$p" ]; then
        echo "ERROR: scan path missing: $p" >&2
        exit 2
    fi
done

# Prefer ripgrep. Both engines scan the same Rust-only, hidden, ignored scope.
if command -v rg >/dev/null 2>&1; then
    GREP() {
        rg --no-heading --line-number --color=never --no-ignore --hidden \
            --text --ignore-case --glob '*.rs' "$@"
    }
else
    GREP() { grep -rHnaiE --color=never --include='*.rs' "$@"; }
fi

HARD_HITS=0
SOFT_HITS=0

print_section() {
    local title="$1"; shift
    local pattern="$1"; shift
    local severity="$1"; shift
    echo
    echo "── $title ──────────────────────────────────────────────"
    local hits raw status
    # Filter out lines carrying the allow-empty-sentinel marker.
    raw=$(GREP "$pattern" "${SCAN_PATHS[@]}")
    status=$?
    case "$status" in
        0|1) ;;
        *) echo "ERROR: sentinel search engine failed (exit $status)" >&2; exit 2 ;;
    esac
    hits=""
    if [ -n "$raw" ]; then
        hits=$(printf '%s\n' "$raw" | grep -vi 'allow-empty-sentinel' || true)
    fi
    if [ -n "$hits" ]; then
        echo "$hits"
        local n
        n="$(printf '%s\n' "$hits" | wc -l | tr -d ' ')"
        if [ "$severity" = hard ]; then
            HARD_HITS=$((HARD_HITS + n))
        else
            SOFT_HITS=$((SOFT_HITS + n))
        fi
    else
        echo "  (no hits)"
    fi
}

# 1) Explicit textual sentinel literals. These are unambiguous offenders.
print_section \
    'Textual sentinel literals ("<unknown>" / "<empty>" / "<none>")' \
    '"(<unknown>|<empty>|<none>)"' \
    hard

# 2) `"".to_string()` used as a default — strong signal of an empty-string
#    sentinel. Legitimate uses (e.g. seeding a buffer) should be rare in
#    route code; mark them with `// allow-empty-sentinel: <reason>`.
print_section \
    '`"".to_string()` defaults' \
    '""\.to_string\(\)' \
    hard

# 3) `.is_empty()` on a `String` field used to mean "unset". This is the
#    high-false-positive bucket — the reviewer must judge each hit. Common
#    legitimate cases: input validation in a handler entry, length checks
#    on Vec, Path components.
print_section \
    '`.is_empty()` calls (review for unset-sentinel semantics)' \
    '\.is_empty\(\)' \
    soft

# 4) `unwrap_or_default()` on Option<String> in handler return paths is a
#    soft signal that an Option got flattened to "" before serialization.
#    Surfaced for review only.
print_section \
    '`unwrap_or_default()` on String-shaped Option (soft signal)' \
    '\.unwrap_or_default\(\)' \
    soft

echo
echo "─────────────────────────────────────────────────────────────"
echo "Hard sentinel hits: $HARD_HITS"
echo "Soft review-only hits: $SOFT_HITS"
echo "Policy: docs/architecture/api-conventions.md"
echo "Suppress a benign hit by appending  // allow-empty-sentinel: <reason>"
echo "─────────────────────────────────────────────────────────────"

if [ "$STRICT" = "1" ] && [ "$HARD_HITS" -gt 0 ]; then
    echo "FAIL (--strict): $HARD_HITS hard sentinel-pattern hits found." >&2
    exit 1
fi

exit 0
