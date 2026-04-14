#!/usr/bin/env bash
#
# Compute area / type labels for a librefang issue based on its title.
# Used by .github/workflows/issue-auto-label.yml — both the event-driven
# path (single new/edited issue) and the workflow_dispatch backfill path
# (re-label all unlabeled open issues).
#
# Usage:
#   auto-label-issue.sh <issue_number> <title> [body_file]
#
# Output (stdout): a comma-separated list of label names, no leading
# comma, no trailing newline. Empty output means "no labels to apply".
#
# Design notes
# - Title-only scan. Scanning the body too proved disastrous on the
#   first backfill pass: bug reports include code blocks, file paths
#   (`crates/librefang-api/...`, `crates/librefang-runtime/...`), and
#   stack traces, so every body grep tagged half a dozen unrelated
#   `area/*` labels. The body_file argument is accepted for backward
#   compatibility but ignored.
# - Each rule sets a `matched` flag. If nothing matched after every rule
#   has run, the script falls back to `needs-triage` so maintainers can
#   spot orphaned issues in the list view.
# - Conventional-commit-style title prefixes are matched as well, but
#   chore: / refactor: / test: only set the matched flag without adding
#   a label (they're meta, not category).
# - Keyword regexes use `\b` word boundaries where the keyword is short
#   enough to false-positive on substrings (e.g. `\bcli\b` so it doesn't
#   trip on `client`).

set -euo pipefail

issue_number="${1:-}"
title="${2:-}"
# body_file argument intentionally ignored — see header notes
_=${3:-}

if [ -z "$issue_number" ] || [ -z "$title" ]; then
  echo "usage: $0 <issue_number> <title> [body_file]" >&2
  exit 2
fi

title_lower=$(printf '%s' "$title" | tr '[:upper:]' '[:lower:]')

labels=""
matched=0

# ── Conventional commit prefix → type label (title only) ─────────────
case "$title_lower" in
  feat:*|'feat('*|feat!:*)
    labels="$labels,enhancement"; matched=1 ;;
  fix:*|'fix('*|fix!:*)
    labels="$labels,bug"; matched=1 ;;
  perf:*|'perf('*)
    labels="$labels,enhancement"; matched=1 ;;
  docs:*|doc:*|'docs('*)
    labels="$labels,area/docs"; matched=1 ;;
  ci:*|build:*|'ci('*|'build('*)
    labels="$labels,area/ci"; matched=1 ;;
  refactor:*|chore:*|test:*|'refactor('*|'chore('*|'test('*)
    matched=1 ;;
esac

# ── Keyword → area label (title only) ────────────────────────────────
add_label_if_match() {
  local pattern="$1"
  local label="$2"
  if printf '%s' "$title_lower" | grep -qiE -- "$pattern"; then
    labels="$labels,$label"
    matched=1
  fi
}

# Keyword regexes use a left-anchored \b only (no trailing \b) for words
# that commonly appear glued to a suffix in librefang's domain vocabulary
# — `MemoryUpdate`, `EventBus`, `TaskBoard`, `tool_use`, `tool_result`,
# `EmbeddingStore`, etc. The leading \b still prevents substring hits
# like `remembered` (no \b before `mem`) or `subtask` (no \b before
# `task`). Words with high suffix FP risk (`hand`, `auth`) keep the
# trailing \b.
add_label_if_match 'channel|telegram|discord|slack|whatsapp|feishu|webhook|messaging' 'area/channels'
add_label_if_match 'skill|fanghub|marketplace' 'area/skills'
add_label_if_match 'kernel|scheduler|cron|rbac|workflow|trigger|\bevent|\btask|session|\bhand\b|spawn' 'area/kernel'
add_label_if_match 'runtime|agent.?loop|\bllm\b|wasm|sandbox|driver|provider|\bmcp\b|\btool[._ -]?(call|use|result)|prompt' 'area/runtime'
add_label_if_match '\bapi\b|endpoint|dashboard|frontend|\bui\b|react|http|rest|websocket|\broute\b' 'area/api'
add_label_if_match '\bsdk\b|python|javascript|typescript|\bnpm\b|\bpip\b' 'area/sdk'
add_label_if_match '\bmemory|knowledge|vector|embedding|sqlite' 'area/memory'
add_label_if_match 'security|\bauth\b|\btoken\b|vulnerability|audit|taint|capability|approval|totp|\bcve\b|sandbox.escape' 'area/security'
add_label_if_match 'docker|deploy|github.?action|gh.?action|pipeline|\bbuild\b' 'area/ci'
add_label_if_match 'documentation|readme|\bguide\b|tutorial|translat|i18n' 'area/docs'
add_label_if_match '\bcli\b|\btui\b' 'area/cli'
add_label_if_match 'tauri|desktop.?app' 'area/desktop'
add_label_if_match 'translat|i18n|chinese|japanese|korean' 'no-rust-required'

# ── Fallback ────────────────────────────────────────────────────────
if [ "$matched" -eq 0 ]; then
  labels="needs-triage"
fi

# ── Strip leading comma + dedupe + drop empties ─────────────────────
printf '%s' "${labels#,}" \
  | tr ',' '\n' \
  | grep -v '^$' \
  | sort -u \
  | paste -sd, -
