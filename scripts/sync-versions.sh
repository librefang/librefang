#!/usr/bin/env bash
#
# sync-versions.sh — Sync version strings across the LibreFang monorepo.
#
# Reads the canonical version from workspace Cargo.toml and updates:
#   - agents/*/agent.toml
#   - sdk/javascript/package.json
#   - sdk/python/setup.py
#   - packages/whatsapp-gateway/package.json
#
# Usage:
#   ./scripts/sync-versions.sh          # sync to current Cargo.toml version
#   ./scripts/sync-versions.sh 0.5.0    # override with explicit version

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ $# -ge 1 ]; then
    VERSION="$1"
else
    VERSION=$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')
fi

echo "Syncing version to: $VERSION"

# --- Agent templates ---
count=0
for f in "$REPO_ROOT"/agents/*/agent.toml; do
    if [ -f "$f" ]; then
        sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" "$f" && rm -f "$f.bak"
        count=$((count + 1))
    fi
done
echo "  Updated $count agent templates"

# --- JavaScript SDK ---
JS_PKG="$REPO_ROOT/sdk/javascript/package.json"
if [ -f "$JS_PKG" ]; then
    sed -i.bak "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" "$JS_PKG" && rm -f "$JS_PKG.bak"
    echo "  Updated sdk/javascript/package.json"
fi

# --- Python SDK ---
PY_SETUP="$REPO_ROOT/sdk/python/setup.py"
if [ -f "$PY_SETUP" ]; then
    sed -i.bak "s/version=\".*\"/version=\"$VERSION\"/" "$PY_SETUP" && rm -f "$PY_SETUP.bak"
    echo "  Updated sdk/python/setup.py"
fi

# --- WhatsApp gateway ---
WA_PKG="$REPO_ROOT/packages/whatsapp-gateway/package.json"
if [ -f "$WA_PKG" ]; then
    sed -i.bak "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" "$WA_PKG" && rm -f "$WA_PKG.bak"
    echo "  Updated packages/whatsapp-gateway/package.json"
fi

echo "Done. Run 'git diff' to review changes."
