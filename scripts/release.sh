#!/bin/bash
# Release script for LibreFang

set -e

PREV_TAG=$(git tag --sort=-creatordate | head -1)
if [ -z "$PREV_TAG" ]; then
    echo "No previous tag found."
    exit 1
fi

PREV_VERSION=$(echo "$PREV_TAG" | sed 's/^v//' | sed 's/-.*//')
MAJOR=$(echo "$PREV_VERSION" | cut -d. -f1)
MINOR=$(echo "$PREV_VERSION" | cut -d. -f2)
PATCH=$(echo "$PREV_VERSION" | cut -d. -f3)

V_PATCH="${MAJOR}.${MINOR}.$((PATCH + 1))"
V_MINOR="${MAJOR}.$((MINOR + 1)).0"
V_MAJOR="$((MAJOR + 1)).0.0"

echo ""
echo "Current: $PREV_VERSION ($PREV_TAG)"
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

DATE=$(date +%Y%m%d)
TAG="v${VERSION}-${DATE}"

echo ""
echo "  $PREV_VERSION → $VERSION ($TAG)"
read -rp "Confirm? [Y/n]: " confirm
if [[ "$confirm" =~ ^[Nn] ]]; then
    echo "Aborted."
    exit 0
fi

# Update version in Cargo.toml
sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
rm -f Cargo.toml.bak

# Refresh lockfile
cargo update --workspace
git add Cargo.toml Cargo.lock

# Delete local and remote tag if exists
git tag -d "$TAG" 2>/dev/null || true
git push origin ":refs/tags/$TAG" 2>/dev/null || true

# Create and push tag
git commit -m "chore: bump version to $TAG"
git tag "$TAG"
git push origin main && git push origin "$TAG"

# Create GitHub Release with auto-generated notes
if command -v gh &>/dev/null; then
    gh release create "$TAG" \
        --repo librefang/librefang \
        --title "LibreFang $TAG" \
        --generate-notes \
        || echo "Warning: gh release create failed — CI will create it"
    echo "→ https://github.com/librefang/librefang/releases/tag/$TAG"
fi

echo ""
echo "Release $TAG done!"
