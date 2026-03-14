#!/usr/bin/env bash
#
# sync-versions.sh — Sync version strings across the LibreFang monorepo.
#
# Usage:
#   ./scripts/sync-versions.sh          # show current version, sync non-Cargo files to it
#   ./scripts/sync-versions.sh 0.5.0    # bump everything to 0.5.0 (including Cargo.toml)
#
# What it updates:
#   - Cargo.toml workspace version (only when explicit version given)
#   - agents/*/agent.toml
#   - sdk/javascript/package.json
#   - sdk/python/setup.py
#   - packages/whatsapp-gateway/package.json

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Extract current workspace version from [workspace.package] section
current_version() {
    awk '
        /^\[workspace\.package\]/ { found=1; next }
        found && /^version/ {
            # extract value between quotes
            match($0, /"[^"]+"/)
            print substr($0, RSTART+1, RLENGTH-2)
            exit
        }
        found && /^\[/ { exit }
    ' "$REPO_ROOT/Cargo.toml"
}

CURRENT=$(current_version)
if [ -z "$CURRENT" ]; then
    echo "Error: could not read version from Cargo.toml" >&2
    exit 1
fi

if [ $# -ge 1 ]; then
    VERSION="$1"
    # Validate semver format
    if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
        echo "Error: '$VERSION' is not a valid semver (expected: X.Y.Z or X.Y.Z-suffix)" >&2
        exit 1
    fi
    if [ "$VERSION" = "$CURRENT" ]; then
        echo "Version is already $VERSION"
    else
        echo "Bumping version: $CURRENT -> $VERSION"
        # Update Cargo.toml workspace version — works on both macOS and Linux
        sed -i.bak '/^\[workspace\.package\]/,/^\[/ s/^version = "'"$CURRENT"'"/version = "'"$VERSION"'"/' \
            "$REPO_ROOT/Cargo.toml" && rm -f "$REPO_ROOT/Cargo.toml.bak"
        echo "  Updated Cargo.toml workspace version"
    fi
else
    VERSION="$CURRENT"
    echo "Syncing to current version: $VERSION"
fi

# --- Agent templates ---
count=0
for f in "$REPO_ROOT"/agents/*/agent.toml; do
    [ -f "$f" ] || continue
    sed -i.bak 's/^version = "[^"]*"/version = "'"$VERSION"'"/' "$f" && rm -f "$f.bak"
    count=$((count + 1))
done
echo "  Updated $count agent templates"

# --- JavaScript SDK (only the top-level "version" field, indented with 2 spaces) ---
JS_PKG="$REPO_ROOT/sdk/javascript/package.json"
if [ -f "$JS_PKG" ]; then
    sed -i.bak 's/^  "version": "[^"]*"/  "version": "'"$VERSION"'"/' "$JS_PKG" && rm -f "$JS_PKG.bak"
    echo "  Updated sdk/javascript/package.json"
fi

# --- Python SDK ---
PY_SETUP="$REPO_ROOT/sdk/python/setup.py"
if [ -f "$PY_SETUP" ]; then
    sed -i.bak 's/version="[^"]*"/version="'"$VERSION"'"/' "$PY_SETUP" && rm -f "$PY_SETUP.bak"
    echo "  Updated sdk/python/setup.py"
fi

# --- WhatsApp gateway (only the top-level "version" field) ---
WA_PKG="$REPO_ROOT/packages/whatsapp-gateway/package.json"
if [ -f "$WA_PKG" ]; then
    sed -i.bak 's/^  "version": "[^"]*"/  "version": "'"$VERSION"'"/' "$WA_PKG" && rm -f "$WA_PKG.bak"
    echo "  Updated packages/whatsapp-gateway/package.json"
fi

# --- Verify ---
echo ""
echo "Verification:"
echo "  Cargo.toml:      $(current_version)"
grep -h '^version' "$REPO_ROOT"/agents/hello-world/agent.toml 2>/dev/null | head -1 | sed 's/^/  agents:          /'
grep '"version"' "$JS_PKG" 2>/dev/null | head -1 | sed 's/^[[:space:]]*/  JS SDK:          /'
grep 'version=' "$PY_SETUP" 2>/dev/null | head -1 | sed 's/^[[:space:]]*/  Python SDK:      /'
echo ""
echo "Done. Run 'git diff' to review changes."
