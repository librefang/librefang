#!/usr/bin/env bash
#
# release.sh — Create a new LibreFang release using CalVer (YYYY.M.DDHH).
#
# Usage:
#   ./scripts/release.sh                    # interactive: choose stable/beta/rc
#   ./scripts/release.sh 2026.3.2114        # explicit stable version
#   ./scripts/release.sh 2026.3.2114-beta1  # explicit pre-release version
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

if [ $# -ge 1 ]; then
    VERSION="$1"
else
    # CalVer: YYYY.M.DDHH
    YEAR=$(date +%Y)
    MONTH=$(date +%-m)
    DAY=$(date +%d)
    HOUR=$(date +%H)
    BASE_VERSION="${YEAR}.${MONTH}.${DAY}${HOUR}"

    # Count existing beta/rc tags for today to auto-increment
    TODAY_BETA_COUNT=$(git -C "$REPO_ROOT" tag -l "v${BASE_VERSION}-beta*" 2>/dev/null | wc -l | tr -d ' ')
    TODAY_RC_COUNT=$(git -C "$REPO_ROOT" tag -l "v${BASE_VERSION}-rc*" 2>/dev/null | wc -l | tr -d ' ')
    NEXT_BETA=$((TODAY_BETA_COUNT + 1))
    NEXT_RC=$((TODAY_RC_COUNT + 1))

    echo ""
    echo "Current version: $CURRENT (tag: ${PREV_TAG:-none})"
    echo ""
    echo "  1) stable  → $BASE_VERSION"
    echo "  2) beta    → ${BASE_VERSION}-beta${NEXT_BETA}"
    echo "  3) rc      → ${BASE_VERSION}-rc${NEXT_RC}"
    echo ""
    read -rp "Choose [1/2/3]: " choice
    case "$choice" in
        1) VERSION="$BASE_VERSION" ;;
        2) VERSION="${BASE_VERSION}-beta${NEXT_BETA}" ;;
        3) VERSION="${BASE_VERSION}-rc${NEXT_RC}" ;;
        *) echo "Invalid choice"; exit 1 ;;
    esac
fi

# Validate CalVer format: YYYY.M.DDHH with optional -betaN or -rcN
if ! echo "$VERSION" | grep -qE '^[0-9]{4}\.[0-9]{1,2}\.[0-9]{2,4}(-(beta|rc)[0-9]+)?$'; then
    echo "Error: '$VERSION' is not a valid CalVer (expected: YYYY.M.DDHH or YYYY.M.DDHH-rc1)" >&2
    exit 1
fi

TAG="v${VERSION}"
# Check if this is a pre-release
IS_PRERELEASE=false
if echo "$VERSION" | grep -qE -- '-(beta|rc)[0-9]'; then
    IS_PRERELEASE=true
fi

echo ""
echo "  Version: $CURRENT → $VERSION"
echo "  Tag:     $TAG"
if [ "$IS_PRERELEASE" = true ]; then
    echo "  Type:    pre-release"
fi
if [ -n "$PREV_TAG" ]; then
    echo "  Review:  https://github.com/librefang/librefang/compare/${PREV_TAG}...main"
fi
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

# --- Extract base version for CHANGELOG matching ---
# Strip pre-release suffix and hour for changelog section
# e.g. 2026.3.2114-beta1 → 2026.3.21
BASE_FOR_CHANGELOG=$(echo "$VERSION" | sed 's/-.*//')
PATCH_PART=$(echo "$BASE_FOR_CHANGELOG" | cut -d. -f3)
if [ ${#PATCH_PART} -eq 4 ]; then
    CHANGELOG_VERSION="$(echo "$BASE_FOR_CHANGELOG" | cut -d. -f1,2).${PATCH_PART:0:2}"
else
    CHANGELOG_VERSION="$BASE_FOR_CHANGELOG"
fi

# --- Generate changelog ---

CHANGELOG_SCRIPT="$REPO_ROOT/scripts/generate-changelog.sh"
if [ -x "$CHANGELOG_SCRIPT" ]; then
    echo ""
    echo "Generating changelog..."
    "$CHANGELOG_SCRIPT" "$CHANGELOG_VERSION" "${PREV_TAG:-}"
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

# --- Generate Dev.to release article (skip for pre-releases) ---

ARTICLE="$REPO_ROOT/articles/release-${CHANGELOG_VERSION}.md"
if [ "$IS_PRERELEASE" = true ]; then
    echo ""
    echo "Skipping Dev.to article for pre-release"
elif [ ! -f "$ARTICLE" ]; then
    CHANGES=$(awk '/^## \['"$CHANGELOG_VERSION"'\]/{found=1; next} found && /^## \[/{exit} found{print}' "$REPO_ROOT/CHANGELOG.md")
    if [ -n "$CHANGES" ]; then
        echo "Generating Dev.to article..."
        cat > "$ARTICLE" <<ARTICLE_EOF
---
title: "LibreFang $CHANGELOG_VERSION Released"
published: true
description: "LibreFang v${CHANGELOG_VERSION} release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/${TAG}
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang $CHANGELOG_VERSION Released

We're excited to announce **LibreFang v${CHANGELOG_VERSION}**! Here's what's new:

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
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
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

# --- Build React dashboard ---

DASHBOARD_DIR="$REPO_ROOT/crates/librefang-api/dashboard"
if [ -f "$DASHBOARD_DIR/package.json" ]; then
    echo ""
    echo "Building React dashboard..."
    (cd "$DASHBOARD_DIR" && pnpm install --frozen-lockfile && pnpm run build)
    echo "  ✓ Dashboard built"
fi

# --- Commit and tag ---

git -C "$REPO_ROOT" add \
    Cargo.toml Cargo.lock \
    CHANGELOG.md \
    sdk/javascript/package.json \
    sdk/python/setup.py \
    sdk/rust/Cargo.toml \
    sdk/rust/README.md \
    packages/whatsapp-gateway/package.json \
    crates/librefang-desktop/tauri.conf.json \
    crates/librefang-api/static/react/
[ -f "$ARTICLE" ] && git -C "$REPO_ROOT" add "$ARTICLE"

if git -C "$REPO_ROOT" diff --cached --quiet; then
    echo ""
    echo "No file changes (re-release). Tagging current HEAD."
else
    git -C "$REPO_ROOT" commit -m "chore: bump version to $TAG"
fi
git -C "$REPO_ROOT" tag "$TAG"

echo ""
echo "Created tag $TAG"

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
    RELEASE_BODY=$(awk '/^## \['"$CHANGELOG_VERSION"'\]/{found=1; next} found && /^## \[/{exit} found{print}' "$REPO_ROOT/CHANGELOG.md")
    PR_BODY="## Release $TAG"
    if [ -n "$RELEASE_BODY" ]; then
        PR_BODY="$PR_BODY

$RELEASE_BODY"
    fi
    if [ -n "$PREV_TAG" ]; then
        PR_BODY="$PR_BODY

---
**Full diff:** https://github.com/librefang/librefang/compare/${PREV_TAG}...${TAG}"
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
