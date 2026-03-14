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

# Must be on main (or we'll create a branch from it)
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
FULL_VERSION="${VERSION}-${DATE}"
TAG="v${FULL_VERSION}"

echo ""
echo "  Version: $CURRENT → $FULL_VERSION"
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
"$SYNC_SCRIPT" "$FULL_VERSION"

# --- Update lockfile if cargo is available ---

if command -v cargo &>/dev/null; then
    echo "Updating Cargo.lock..."
    cargo update --workspace 2>/dev/null || echo "Warning: cargo update failed, continuing"
fi

# --- Commit and tag ---

git -C "$REPO_ROOT" add \
    Cargo.toml Cargo.lock \
    CHANGELOG.md \
    sdk/javascript/package.json \
    sdk/python/setup.py \
    packages/whatsapp-gateway/package.json
git -C "$REPO_ROOT" commit -m "chore: bump version to $TAG"
git -C "$REPO_ROOT" tag "$TAG"

echo ""
echo "Created commit and tag $TAG"

# --- Create branch and push ---

RELEASE_BRANCH="chore/bump-version-${VERSION}"

echo ""
echo "Creating release branch '$RELEASE_BRANCH'..."
git -C "$REPO_ROOT" checkout -b "$RELEASE_BRANCH"

read -rp "Push and create PR? [Y/n]: " push_confirm
if [[ "$push_confirm" =~ ^[Nn] ]]; then
    echo "Skipped push. Run manually:"
    echo "  git push -u origin $RELEASE_BRANCH"
    echo "  gh pr create --title 'chore: bump version to $TAG'"
    exit 0
fi

git -C "$REPO_ROOT" push -u origin "$RELEASE_BRANCH"
git -C "$REPO_ROOT" push origin "$TAG"

# --- Create PR ---

if command -v gh &>/dev/null; then
    echo ""
    echo "Creating Pull Request..."

    # Extract the current version's section from CHANGELOG.md as PR body
    RELEASE_BODY=$(awk '/^## \['"$VERSION"'\]/{found=1; next} found && /^## \[/{exit} found{print}' "$REPO_ROOT/CHANGELOG.md")
    PR_BODY="## Release $TAG"
    if [ -n "$RELEASE_BODY" ]; then
        PR_BODY="$PR_BODY

$RELEASE_BODY"
    fi

    PR_URL=$(gh pr create \
        --repo librefang/librefang \
        --title "release: $TAG" \
        --body "$PR_BODY" \
        --base main \
        --head "$RELEASE_BRANCH")

    echo "→ $PR_URL"
else
    echo ""
    echo "gh CLI not found. Create a PR manually for branch '$RELEASE_BRANCH'."
fi

echo ""
echo "Tag $TAG pushed — release.yml workflow will auto-create the GitHub Release."
echo "Merge the PR to land the version bump on main."
