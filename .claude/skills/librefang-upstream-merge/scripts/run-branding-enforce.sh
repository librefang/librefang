#!/usr/bin/env bash
# run-branding-enforce.sh — Phase 3d of the librefang-upstream-merge skill.
#
# Runs the repo's brand-enforcement script in fix-then-audit mode.
#
# Step 1: `enforce-branding.py` (idempotent token replacement). Edits the
#         tree to flip any upstream sky-blue tokens back to BossFang ember.
# Step 2: `enforce-branding.py --check` (read-only audit). Exits 1 if any
#         upstream token survives the replacement pass — typically a new
#         SVG fang glyph that needs manual replacement with
#         `<img src="/boss-libre.png" alt="BossFang" ...>`.
#
# Both invocations are wrapped so the script itself stays exit-code 0
# even when the audit flags something (the audit's stderr already
# names every offending file and pattern). The caller (the skill's
# Phase 3) decides whether to block on the audit.

set -uo pipefail

toplevel="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
script="$toplevel/scripts/enforce-branding.py"

if [ ! -x "$script" ] && [ ! -f "$script" ]; then
  echo "[brand-enforce] FAIL: $script missing" >&2
  exit 1
fi

echo "[brand-enforce] step 1: applying token replacements (idempotent)..."
python3 "$script"
step1_status=$?

echo
echo "[brand-enforce] step 2: read-only audit for surviving upstream tokens..."
python3 "$script" --check
audit_status=$?

echo
if [ "$audit_status" -eq 0 ]; then
  echo "[brand-enforce] ok — no upstream tokens detected"
  exit 0
else
  echo "[brand-enforce] audit flagged $audit_status — manual fix required (typically a new SVG fang glyph or hardcoded sky-blue rgba outside the script's pattern list). See references/branding-tokens.md."
  exit 1
fi
