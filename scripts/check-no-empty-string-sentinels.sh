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

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STRICT=0
for arg in "$@"; do
    case "$arg" in
        --strict) STRICT=1 ;;
        -h|--help)
            sed -n '2,30p' "$0"
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

# Pick a grep that supports PCRE-ish features. Prefer ripgrep if available
# (every dev machine has it; CI installs it via the rust-cache action).
if command -v rg >/dev/null 2>&1; then
    GREP() { rg --no-heading --line-number --color=never "$@"; }
else
    GREP() { grep -rnE --color=never "$@"; }
fi

TOTAL_HITS=0

print_section() {
    local title="$1"; shift
    local pattern="$1"; shift
    echo
    echo "── $title ──────────────────────────────────────────────"
    local hits
    # Filter out lines carrying the allow-empty-sentinel marker.
    hits="$(GREP "$pattern" "${SCAN_PATHS[@]}" 2>/dev/null \
        | grep -v 'allow-empty-sentinel' \
        || true)"
    if [ -n "$hits" ]; then
        echo "$hits"
        local n
        n="$(printf '%s\n' "$hits" | wc -l | tr -d ' ')"
        TOTAL_HITS=$((TOTAL_HITS + n))
    else
        echo "  (no hits)"
    fi
}

# 1) Explicit textual sentinel literals. These are unambiguous offenders.
print_section \
    'Textual sentinel literals ("<unknown>" / "<empty>" / "<none>" / bare "none")' \
    '"(<unknown>|<empty>|<none>)"|=> "none"|: "none"'

# 2) `"".to_string()` used as a default — strong signal of an empty-string
#    sentinel. Legitimate uses (e.g. seeding a buffer) should be rare in
#    route code; mark them with `// allow-empty-sentinel: <reason>`.
print_section \
    '`"".to_string()` defaults' \
    '""\.to_string\(\)'

# 3) `.is_empty()` on a `String` field used to mean "unset". This is the
#    high-false-positive bucket — the reviewer must judge each hit. Common
#    legitimate cases: input validation in a handler entry, length checks
#    on Vec, Path components.
print_section \
    '`.is_empty()` calls (review for unset-sentinel semantics)' \
    '\.is_empty\(\)'

# 4) `unwrap_or_default()` on Option<String> in handler return paths is a
#    soft signal that an Option got flattened to "" before serialization.
#    Surfaced for review only.
print_section \
    '`unwrap_or_default()` on String-shaped Option (soft signal)' \
    '\.unwrap_or_default\(\)'

echo
echo "─────────────────────────────────────────────────────────────"
echo "Total candidate hits: $TOTAL_HITS"
echo "Policy: docs/architecture/api-conventions.md"
echo "Suppress a benign hit by appending  // allow-empty-sentinel: <reason>"
echo "─────────────────────────────────────────────────────────────"

if [ "$STRICT" = "1" ] && [ "$TOTAL_HITS" -gt 0 ]; then
    echo "FAIL (--strict): $TOTAL_HITS sentinel-pattern hits found." >&2
    exit 1
fi

exit 0
