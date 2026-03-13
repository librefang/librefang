#!/bin/bash
# Release script for LibreFang

set -e

# Get version from command line
if [ -z "$1" ]; then
    echo "Usage: $0 <version>"
    echo "  Example: $0 0.3.49"
    exit 1
fi

VERSION="$1"
DATE=$(date +%Y%m%d)
TAG="v${VERSION}-${DATE}"

echo "Creating release: $TAG"

# Update version in Cargo.toml
sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
rm -f Cargo.toml.bak

# Refresh lockfile so workspace package versions stay in sync with Cargo.toml.
cargo update --workspace
git add Cargo.toml Cargo.lock

# Delete local and remote tag if exists
git tag -d $TAG 2>/dev/null || true
git push origin :refs/tags/$TAG 2>/dev/null || true

# Create and push tag
git commit -m "chore: bump version to $TAG"
git tag $TAG
git push origin main && git push origin $TAG

echo "Release $TAG triggered!"
