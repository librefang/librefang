#!/usr/bin/env bash
#
# sync-versions.sh — Sync version strings across the LibreFang monorepo.
#
# Usage:
#   ./scripts/sync-versions.sh              # show current version, sync non-Cargo files to it
#   ./scripts/sync-versions.sh 2026.3.2114  # bump everything to 2026.3.2114 (including Cargo.toml)
#
# What it updates:
#   - Cargo.toml workspace version (only when explicit version given)
#   - crates/librefang-desktop/tauri.conf.json
#   - sdk/javascript/package.json
#   - sdk/python/setup.py
#   - sdk/rust/Cargo.toml
#   - sdk/rust/README.md
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
    # Validate CalVer format: YYYY.M.DDHH with optional -betaN or -rcN
    if ! echo "$VERSION" | grep -qE '^[0-9]{4}\.[0-9]{1,2}\.[0-9]{2,4}(-(beta|rc)[0-9]+)?$'; then
        echo "Error: '$VERSION' is not a valid CalVer (expected: YYYY.M.DDHH e.g. 2026.3.2114)" >&2
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

# --- JavaScript SDK (only the top-level "version" field, indented with 2 spaces) ---
JS_PKG="$REPO_ROOT/sdk/javascript/package.json"
if [ -f "$JS_PKG" ]; then
    sed -i.bak 's/^  "version": "[^"]*"/  "version": "'"$VERSION"'"/' "$JS_PKG" && rm -f "$JS_PKG.bak"
    echo "  Updated sdk/javascript/package.json"
fi

# --- Rust SDK ---
RS_CARGO="$REPO_ROOT/sdk/rust/Cargo.toml"
if [ -f "$RS_CARGO" ]; then
    sed -i.bak '/^\[package\]/,/^\[/ s/^version = "[^"]*"/version = "'"$VERSION"'"/' "$RS_CARGO" && rm -f "$RS_CARGO.bak"
    echo "  Updated sdk/rust/Cargo.toml"
fi

# --- Rust SDK README (dependency version uses MAJOR.MINOR) ---
RS_README="$REPO_ROOT/sdk/rust/README.md"
if [ -f "$RS_README" ]; then
    MAJOR_MINOR=$(echo "$VERSION" | cut -d. -f1,2)
    sed -i.bak '/^\[dependencies\]/,/^```/ s/librefang = "[^"]*"/librefang = "'"$MAJOR_MINOR"'"/' "$RS_README" && rm -f "$RS_README.bak"
    echo "  Updated sdk/rust/README.md"
fi

# --- Python SDK (PEP 440: -beta1 → b1, -rc1 → rc1) ---
PY_SETUP="$REPO_ROOT/sdk/python/setup.py"
if [ -f "$PY_SETUP" ]; then
    PY_VERSION=$(echo "$VERSION" | sed 's/-beta/b/; s/-rc/rc/')
    sed -i.bak 's/version="[^"]*"/version="'"$PY_VERSION"'"/' "$PY_SETUP" && rm -f "$PY_SETUP.bak"
    echo "  Updated sdk/python/setup.py (PEP 440: $PY_VERSION)"
fi

# --- WhatsApp gateway (only the top-level "version" field) ---
WA_PKG="$REPO_ROOT/packages/whatsapp-gateway/package.json"
if [ -f "$WA_PKG" ]; then
    sed -i.bak 's/^  "version": "[^"]*"/  "version": "'"$VERSION"'"/' "$WA_PKG" && rm -f "$WA_PKG.bak"
    echo "  Updated packages/whatsapp-gateway/package.json"
fi

# --- Tauri desktop app (MSI requires numeric X.Y.Z, each component ≤ 65535,
#     and WiX additionally requires Major ≤ 255) ---
# Use 2-digit year (YY) instead of YYYY so Major ≤ 99.
# Encode pre-release into patch so beta/rc/stable each get a unique version:
#   beta N → DDHH*10 + N       (e.g. beta1=21141, beta2=21142)
#   rc N   → DDHH*10 + 4 + N   (e.g. rc1=21145, rc2=21146)
#   stable → DDHH*10 + 9       (e.g. 21149)
# Ordering: beta1 < beta4 < rc1 < rc4 < stable.  Max patch: 3123*10+9 = 31239 ≤ 65535.
TAURI_CONF="$REPO_ROOT/crates/librefang-desktop/tauri.conf.json"
if [ -f "$TAURI_CONF" ]; then
    BASE_VER=$(echo "$VERSION" | sed 's/-.*//')
    YYYY=$(echo "$BASE_VER" | cut -d. -f1)
    YY=$((YYYY % 100))
    MONTH=$(echo "$BASE_VER" | cut -d. -f2)
    DDHH=$(echo "$BASE_VER" | cut -d. -f3)
    PRERELEASE=$(echo "$VERSION" | grep -oE '-(beta|rc)([0-9]+)' || true)
    if [ -z "$PRERELEASE" ]; then
        TAURI_PATCH=$((DDHH * 10 + 9))
    elif echo "$PRERELEASE" | grep -q 'beta'; then
        N=$(echo "$PRERELEASE" | grep -oE '[0-9]+')
        TAURI_PATCH=$((DDHH * 10 + N))
    else
        N=$(echo "$PRERELEASE" | grep -oE '[0-9]+')
        TAURI_PATCH=$((DDHH * 10 + 4 + N))
    fi
    TAURI_VERSION="${YY}.${MONTH}.${TAURI_PATCH}"
    sed -i.bak 's/"version": "[^"]*"/"version": "'"$TAURI_VERSION"'"/' "$TAURI_CONF" && rm -f "$TAURI_CONF.bak"
    echo "  Updated crates/librefang-desktop/tauri.conf.json ($TAURI_VERSION)"
fi

# --- Verify ---
echo ""
echo "Verification:"
echo "  Cargo.toml:      $(current_version)"
grep '"version"' "$JS_PKG" 2>/dev/null | head -1 | sed 's/^[[:space:]]*/  JS SDK:          /'
grep 'version=' "$PY_SETUP" 2>/dev/null | head -1 | sed 's/^[[:space:]]*/  Python SDK:      /'
grep '^version' "$RS_CARGO" 2>/dev/null | head -1 | sed 's/^[[:space:]]*/  Rust SDK:        /'
echo ""
echo "Done. Run 'git diff' to review changes."
