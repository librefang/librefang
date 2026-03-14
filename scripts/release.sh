#!/usr/bin/env bash
#
# release.sh — Create a new LibreFang release.
#
# Usage:
#   ./scripts/release.sh            # interactive: choose patch/minor/major
#   ./scripts/release.sh 0.5.0      # explicit version
#
# What it does:
#   1. Validate environment (clean worktree, on main, up to date)
#   2. Bump version via sync-versions.sh (Cargo.toml + agents + SDKs)
#   3. Commit, tag, push
#   4. Create GitHub Release (if gh is available)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SYNC_SCRIPT="$REPO_ROOT/scripts/sync-versions.sh"

# --- Preflight checks ---

if [ ! -x "$SYNC_SCRIPT" ]; then
    echo "Error: sync-versions.sh not found or not executable" >&2
    exit 1
fi

# Must be on main
BRANCH=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ]; then
    echo "Error: must be on 'main' branch (currently on '$BRANCH')" >&2
    exit 1
fi

# Must have clean worktree
if ! git -C "$REPO_ROOT" diff --quiet || ! git -C "$REPO_ROOT" diff --cached --quiet; then
    echo "Error: working tree is dirty. Commit or stash changes first." >&2
    git -C "$REPO_ROOT" status --short
    exit 1
fi

# Pull latest
echo "Pulling latest main..."
git -C "$REPO_ROOT" pull --rebase origin main

# --- Determine version ---

PREV_TAG=$(git -C "$REPO_ROOT" tag --sort=-creatordate | grep -E '^v[0-9]' | head -1 || true)
if [ -z "$PREV_TAG" ]; then
    echo "Warning: no previous version tag found, reading from Cargo.toml"
fi

# Read current version from Cargo.toml (authoritative source)
CURRENT=$(awk '/^\[workspace\.package\]/{f=1;next} f&&/^version/{match($0,/"[^"]+"/);print substr($0,RSTART+1,RLENGTH-2);exit}' "$REPO_ROOT/Cargo.toml")
if [ -z "$CURRENT" ]; then
    echo "Error: could not read version from Cargo.toml" >&2
    exit 1
fi

MAJOR=$(echo "$CURRENT" | cut -d. -f1)
MINOR=$(echo "$CURRENT" | cut -d. -f2)
PATCH=$(echo "$CURRENT" | cut -d. -f3 | sed 's/-.*//')

if [ $# -ge 1 ]; then
    VERSION="$1"
else
    V_PATCH="${MAJOR}.${MINOR}.$((PATCH + 1))"
    V_MINOR="${MAJOR}.$((MINOR + 1)).0"
    V_MAJOR="$((MAJOR + 1)).0.0"

    echo ""
    echo "Current version: $CURRENT (tag: ${PREV_TAG:-none})"
    echo ""
    echo "  1) patch  → $V_PATCH"
    echo "  2) minor  → $V_MINOR"
    echo "  3) major  → $V_MAJOR"
    echo ""
    read -rp "Choose [1/2/3]: " choice
    case "$choice" in
        1) VERSION="$V_PATCH" ;;
        2) VERSION="$V_MINOR" ;;
        3) VERSION="$V_MAJOR" ;;
        *) echo "Invalid choice"; exit 1 ;;
    esac
fi

# Validate semver
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo "Error: '$VERSION' is not a valid semver" >&2
    exit 1
fi

DATE=$(date +%Y%m%d)
TAG="v${VERSION}-${DATE}"

echo ""
echo "  Version: $CURRENT → $VERSION"
echo "  Tag:     $TAG"
echo ""
read -rp "Confirm? [Y/n]: " confirm
if [[ "$confirm" =~ ^[Nn] ]]; then
    echo "Aborted."
    exit 0
fi

# --- Check tag doesn't already exist ---

if git -C "$REPO_ROOT" rev-parse "$TAG" &>/dev/null; then
    echo "Error: tag '$TAG' already exists. Delete it first or choose a different version." >&2
    exit 1
fi

# --- Generate changelog ---

CHANGELOG_SCRIPT="$REPO_ROOT/scripts/generate-changelog.sh"
if [ -x "$CHANGELOG_SCRIPT" ]; then
    echo ""
    echo "Generating changelog..."
    "$CHANGELOG_SCRIPT" "$VERSION" "${PREV_TAG:-}"
fi

# --- Bump all versions ---

echo ""
echo "Syncing versions..."
"$SYNC_SCRIPT" "$VERSION"

# --- Update lockfile if cargo is available ---

if command -v cargo &>/dev/null; then
    echo "Updating Cargo.lock..."
    cargo update --workspace 2>/dev/null || echo "Warning: cargo update failed, continuing"
fi

# --- Commit and tag ---

git -C "$REPO_ROOT" add \
    Cargo.toml Cargo.lock \
    CHANGELOG.md \
    agents/*/agent.toml \
    sdk/javascript/package.json \
    sdk/python/setup.py \
    packages/whatsapp-gateway/package.json
git -C "$REPO_ROOT" commit -m "chore: bump version to $TAG"
git -C "$REPO_ROOT" tag "$TAG"

echo ""
echo "Created commit and tag $TAG"

# --- Push ---

read -rp "Push to origin? [Y/n]: " push_confirm
if [[ "$push_confirm" =~ ^[Nn] ]]; then
    echo "Skipped push. Run manually:"
    echo "  git push origin main && git push origin $TAG"
    exit 0
fi

git -C "$REPO_ROOT" push origin main
git -C "$REPO_ROOT" push origin "$TAG"

# --- GitHub Release ---

if command -v gh &>/dev/null; then
    echo ""
    echo "Creating GitHub Release..."
    # Extract the current version's section from CHANGELOG.md as release body
    RELEASE_BODY=$(awk '/^## \['"$VERSION"'\]/{found=1; next} found && /^## \[/{exit} found{print}' "$REPO_ROOT/CHANGELOG.md")
    if [ -n "$RELEASE_BODY" ]; then
        gh release create "$TAG" \
            --repo librefang/librefang \
            --title "LibreFang $VERSION" \
            --notes "$RELEASE_BODY" \
            || echo "Warning: gh release create failed — CI may create it"
    else
        gh release create "$TAG" \
            --repo librefang/librefang \
            --title "LibreFang $VERSION" \
            --generate-notes \
            || echo "Warning: gh release create failed — CI may create it"
    fi
    echo "→ https://github.com/librefang/librefang/releases/tag/$TAG"
fi

echo ""
echo "Release $TAG done!"
