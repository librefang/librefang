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

PREV_TAG=$(git -C "$REPO_ROOT" tag --sort=-creatordate | grep -E '^v[0-9]' | grep -vE '(alpha|beta|rc)' | head -1 || true)
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
    V_CURRENT="${MAJOR}.${MINOR}.${PATCH}"
    V_PATCH="${MAJOR}.${MINOR}.$((PATCH + 1))"
    V_MINOR="${MAJOR}.$((MINOR + 1)).0"
    V_MAJOR="$((MAJOR + 1)).0.0"

    echo ""
    echo "Current version: $CURRENT (tag: ${PREV_TAG:-none})"
    echo ""
    echo "  1) patch   → $V_PATCH"
    echo "  2) minor   → $V_MINOR"
    echo "  3) major   → $V_MAJOR"
    echo "  4) current → $V_CURRENT (re-release, overwrites existing tag)"
    echo ""
    read -rp "Choose [1/2/3/4]: " choice
    case "$choice" in
        1) VERSION="$V_PATCH" ;;
        2) VERSION="$V_MINOR" ;;
        3) VERSION="$V_MAJOR" ;;
        4) VERSION="$V_CURRENT" ;;
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
    echo ""
    echo "Tag '$TAG' already exists."
    read -rp "Delete and re-create it? [Y/n]: " overwrite_confirm
    if [[ "$overwrite_confirm" =~ ^[Nn] ]]; then
        echo "Aborted."
        exit 0
    fi
    echo "Deleting existing tag '$TAG'..."
    git -C "$REPO_ROOT" tag -d "$TAG"
    git -C "$REPO_ROOT" push origin --delete "$TAG" 2>/dev/null || true

    # Also delete existing release branch if present
    RELEASE_BRANCH_CHECK="chore/bump-version-${VERSION}"
    if git -C "$REPO_ROOT" rev-parse --verify "refs/heads/$RELEASE_BRANCH_CHECK" &>/dev/null; then
        git -C "$REPO_ROOT" branch -D "$RELEASE_BRANCH_CHECK"
    fi
    git -C "$REPO_ROOT" push origin --delete "$RELEASE_BRANCH_CHECK" 2>/dev/null || true

    # Delete existing GitHub release if gh is available
    if command -v gh &>/dev/null; then
        gh release delete "$TAG" --repo librefang/librefang --yes 2>/dev/null || true
    fi

    # Re-fetch PREV_TAG since we just deleted the old one
    PREV_TAG=$(git -C "$REPO_ROOT" tag --sort=-creatordate | grep -E '^v[0-9]' | grep -vE '(alpha|beta|rc)' | head -1 || true)
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

# --- Generate Dev.to release article ---

ARTICLE="$REPO_ROOT/articles/release-${VERSION}.md"
if [ ! -f "$ARTICLE" ]; then
    CHANGES=$(awk '/^## \['"$VERSION"'\]/{found=1; next} found && /^## \[/{exit} found{print}' "$REPO_ROOT/CHANGELOG.md")
    if [ -n "$CHANGES" ]; then
        echo "Generating Dev.to article..."
        cat > "$ARTICLE" <<ARTICLE_EOF
---
title: "LibreFang $VERSION Released"
published: true
description: "LibreFang v${VERSION} release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/${TAG}
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang $VERSION Released

We're excited to announce **LibreFang v${VERSION}**! Here's what's new:

${CHANGES}

## Install / Upgrade

\`\`\`bash
# Binary
curl -fsSL https://get.librefang.ai | sh

# Rust SDK
cargo add librefang

# JavaScript SDK
npm install @librefang/sdk

# Python SDK
pip install librefang-sdk
\`\`\`

## Links

- [Full Changelog](https://github.com/librefang/librefang/blob/main/CHANGELOG.md)
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/${TAG})
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
ARTICLE_EOF

        # Polish article with Claude CLI if available
        if command -v claude &>/dev/null; then
            echo "  Polishing with Claude..."
            POLISHED=$(env -u CLAUDECODE claude -p --model claude-haiku-4-5-20251001 --output-format text "You are writing a Dev.to release announcement for LibreFang, an open-source Agent OS built in Rust.
Rewrite the article body to be more engaging and developer-friendly.
Group related changes, highlight the most impactful ones, and add a brief intro.
Keep the same front matter (--- block), Install/Upgrade section, and Links section exactly as-is.
Only rewrite the content between the front matter and the Install section.
Output the COMPLETE article (front matter + body + install + links), ready to save as-is.

Current article:
$(cat "$ARTICLE")" 2>/dev/null) || true
            if [ -n "$POLISHED" ]; then
                echo "$POLISHED" > "$ARTICLE"
                echo "  ✓ AI polished"
            else
                echo "  ⚠ AI polish failed, using raw changelog"
            fi
        fi

        echo "  Generated $ARTICLE"
    fi
fi

# --- Commit and tag ---

git -C "$REPO_ROOT" add \
    Cargo.toml Cargo.lock \
    CHANGELOG.md \
    sdk/javascript/package.json \
    sdk/python/setup.py \
    packages/whatsapp-gateway/package.json \
    crates/librefang-desktop/tauri.conf.json
[ -f "$ARTICLE" ] && git -C "$REPO_ROOT" add "$ARTICLE"
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
git -C "$REPO_ROOT" push origin "$TAG" --force

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

    # Auto-merge the release PR (squash) once CI passes
    gh pr merge "$PR_URL" --auto --squash --repo librefang/librefang
else
    echo ""
    echo "gh CLI not found. Create a PR manually for branch '$RELEASE_BRANCH'."
fi

echo ""
echo "Tag $TAG pushed — release.yml workflow will auto-create the GitHub Release."
echo "Merge the PR to land the version bump on main."
